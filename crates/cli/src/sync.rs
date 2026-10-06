//! Relay sync: registration, auth, the outbox, presence, and applying incoming blobs.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::app::{remember_applied, upsert_device, App, OutboxItem, Slot};
use crate::events::{Event, RelayProblem};
use crate::keys::unb64;
use crate::model::*;
use crate::relay::{BlobIn, RelayError, Signal};

/// A pull page at least this long is applied as a backlog: see `App::bulk_sync`.
const BULK_BLOBS: usize = 20;

/// What a pull takes. `file` blobs are left out: a transcript fetches them by id when it
/// needs them, so a photo sent to one bot is not downloaded by every Device.
pub const POLL_KINDS: &str = "roster,policy,chat,machine,credentials,job,job_cancel,job_result,request,response";

pub async fn run(app: Arc<App>) {
    let mut failures: u32 = 0;
    // The last try's bearer was refused and the next one went at once: a second refusal in a
    // row waits out the backoff.
    let mut signed_in_again = false;
    loop {
        let wakes = app.sync_wakes.load(Ordering::Relaxed);
        match session(&app, &mut failures).await {
            Ok(()) => {
                failures = 0;
                signed_in_again = false;
            }
            Err(error) => {
                let dropped = disconnected(&app);
                if dropped || !error.is_unauthorized() {
                    signed_in_again = false;
                }
                if error.is_unauthorized() {
                    app.relay.forget_token();
                }
                // Another Device unpaired this one: the relay is done with its key, so its
                // copy of the account goes. Onboarding is next.
                if error.is_unpaired() {
                    tracing::warn!("this Device was unpaired; forgetting the identity");
                    if let Err(error) = app.forget_identity() {
                        tracing::error!(%error, "forgetting the identity");
                    }
                    failures = 0;
                    continue;
                }
                // The relay no longer serves this build. The app says so, and the next try
                // waits: the answer stays the same until Lorca is updated or the relay changes.
                let outdated = error.is_update_required();
                if outdated && !app.relay_update_required.swap(true, Ordering::Relaxed) {
                    app.emit_relay_status();
                }
                // A try that never connected says why, until one does. A socket that worked and
                // then ended says nothing, since the next try usually connects, and neither does
                // a stale bearer, which the next try replaces.
                if !dropped && !outdated && !error.is_unauthorized() {
                    let unknown_machine = error.is_unknown_machine() && !app.is_identity_device();
                    app.relay_failed(RelayProblem { message: error.message.clone(), unknown_machine });
                }
                // A relay that restarted with a new secret refuses the bearer this Device held.
                // The next try signs in again, so it goes at once.
                if error.is_unauthorized() && !std::mem::replace(&mut signed_in_again, true) {
                    tracing::info!(%error, "relay; signing in again");
                    continue;
                }
                // Armed before the check below, so a wake between the two still ends the wait.
                let woken = app.outbox_notify.notified();
                tokio::pin!(woken);
                woken.as_mut().enable();
                // A wake came during this session: the phone is back in the foreground, where a
                // socket that died while the app was suspended is expected. The next one opens
                // now, and the backoff is left for failures after that.
                if app.sync_wakes.load(Ordering::Relaxed) != wakes {
                    tracing::info!(%error, "relay; connecting again after a wake");
                    continue;
                }
                failures = failures.saturating_add(1);
                let delay = if outdated { 900 } else { (2u64.pow(failures.min(5))).min(60) };
                tracing::warn!(%error, retry_in = delay, "relay");
                // Up to a second on top, so the Devices a relay restart dropped together do
                // not all come back in the same instant.
                let jitter = std::time::Duration::from_millis(rand::Rng::gen_range(&mut rand::thread_rng(), 0..1000));
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(delay) + jitter) => {}
                    _ = woken => {}
                }
            }
        }
    }
}

/// Without its own socket this Device knows nothing of the others' presence. True when a
/// socket was open.
fn disconnected(app: &Arc<App>) -> bool {
    let was_connected = app.relay_connected.swap(false, Ordering::Relaxed);
    if was_connected {
        app.emit_relay_status();
    }
    let had_online = {
        let mut state = app.state.lock().unwrap();
        let had = !state.device_online.is_empty();
        state.device_online.clear();
        had
    };
    if had_online {
        app.emit(app.roster_summary());
    }
    was_connected
}

/// One sync socket, from connect to its end. The relay signals over it and carries no data:
/// `blobs` is answered with a pull, `machines` with a fresh machine list. The outbox wakes
/// the session too. `Ok` means the identity or the relay URL changed and the next session
/// starts from there; an error is the socket or a request failing. Each round that goes
/// through sets `failures` back to zero, so a socket that worked for hours before it dropped
/// is followed by the shortest backoff.
async fn session(app: &Arc<App>, failures: &mut u32) -> Result<(), RelayError> {
    let machine_file = {
        // Armed before the check: creating, restoring, or pairing an identity queues a blob, so
        // the first session starts as soon as there is one.
        let joined = app.outbox_notify.notified();
        tokio::pin!(joined);
        joined.as_mut().enable();
        let Some(machine_file) = app.machine_file() else {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                _ = joined => {}
            }
            return Ok(());
        };
        machine_file
    };
    let Some(url) = app.relay_url() else {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
            _ = app.outbox_notify.notified() => {}
        }
        return Ok(());
    };
    // A relay without durable policy blobs must not accept a roster that could erase Pause.
    if app.relay.health(&url).await? < crate::relay::PROTOCOL {
        return Err(RelayError { status: Some(426), message: "Relay update required for encrypted policy sync".into() });
    }
    let machine = machine_file.machine().map_err(|e| RelayError { status: None, message: e.to_string() })?;

    if !machine_file.registered {
        ensure_registered(app, &url).await?;
    }

    // The socket opens before the first pull, so no blob lands unseen between the two.
    let token = token_or_register(app, &url, &machine).await?;
    let mut socket = app.relay.sync_socket(&url, &token).await?;
    let was_refused = app.relay_update_required.swap(false, Ordering::Relaxed);
    let had_problem = app.relay_problem.lock().unwrap().take().is_some();
    if !app.relay_connected.swap(true, Ordering::Relaxed) || was_refused || had_problem {
        app.emit_relay_status();
    }
    {
        let mut state = app.state.lock().unwrap();
        // The other Devices dropped this one's turns when the relay last showed it offline;
        // the first round lists them again.
        state.machine_blob_hash = None;
        // What landed while this Device was away is unread until the first pull ends.
        state.caught_up = false;
    }

    let mut refresh = true;
    let mut credentials_due = true;
    let mut wakes = app.sync_wakes.load(Ordering::Relaxed);
    loop {
        let same_machine = app.machine_file().is_some_and(|file| file.machine().is_ok_and(|m| m.pubkey() == machine.pubkey()));
        if !same_machine || app.relay_url().as_deref() != Some(url.as_str()) {
            disconnected(app);
            return Ok(());
        }
        // Armed before the outbox is read: a blob queued while this round runs wakes the wait
        // below instead of sitting there until the relay next speaks.
        let queued = app.outbox_notify.notified();
        tokio::pin!(queued);
        queued.as_mut().enable();

        let token = token_or_register(app, &url, &machine).await?;
        // Reconcile encrypted policy events before a queued roster can supersede the relay's
        // latest roster. The relay itself cannot inspect either payload.
        let queued_roster = app.store.queued_roster().map_err(|error| RelayError { status: None, message: error.to_string() })?;
        // Check the latest roster slot before applying it: legacy queues have no baseline,
        // and an impossible group merge must not purge local transcripts or upload anything.
        if let Some(item) = &queued_roster {
            if let Err(conflict) = preview_roster_conflict(app, &url, &token, &machine_file, item).await {
                pull_policy_only(app, &url, &token, &machine_file).await?;
                return Err(conflict);
            }
        }
        if let Err(conflict) = pull_blobs(app, &url, &token, &machine_file).await {
            if conflict.message.starts_with("Roster conflict:") {
                pull_policy_only(app, &url, &token, &machine_file).await?;
            }
            return Err(conflict);
        }
        if queued_roster.is_some() {
            if let Some(current) = app.store.queued_roster().map_err(|error| RelayError { status: None, message: error.to_string() })? {
                rebase_queued_roster(app, &machine_file, current)?;
            }
        }
        #[cfg(feature = "runner")]
        app.close_orphan_proposals().map_err(|error| RelayError { status: None, message: error.to_string() })?;
        app.push_machine_blob_if_changed();
        drain_outbox(app, &url, &token).await?;
        drain_group_deletes(app, &url, &token).await?;
        drain_blob_deletes(app, &url, &token).await?;
        if refresh {
            refresh_presence(app, &url, &token).await?;
        }
        if std::mem::take(&mut credentials_due) {
            app.push_credentials_if_owed();
        }
        if app.presence_stale.swap(false, Ordering::Relaxed) {
            refresh_presence(app, &url, &token).await?;
        }
        // After the pull, so a list saved before a restart does not show turns the pull ends.
        app.turns_changed();
        *failures = 0;

        refresh = tokio::select! {
            signal = socket.next() => match signal? {
                Signal::Blobs => false,
                Signal::Machines => true,
            },
            _ = &mut queued => {
                let woken = app.sync_wakes.load(Ordering::Relaxed);
                if woken != wakes {
                    wakes = woken;
                    socket.probe().await?;
                }
                false
            }
        };
    }
}

/// Everything a Device polls for but the messages.
const NOT_CHAT_KINDS: &str = "roster,policy,machine,credentials,job,job_cancel,job_result,request,response";
/// How much of each chat a Device takes when it first syncs: what a bot's turn reads.
const FIRST_SYNC_MESSAGES: usize = 400;
/// Messages to a page when reading a chat backwards.
const OLDER_PAGE: usize = 100;

/// A Device's first sync. The log holds every message of the account, so it is not replayed:
/// the Device takes the rest of the log, which slots keep short, then the newest messages of
/// each chat, and follows the log from where it stood when this began. What landed meanwhile
/// comes again through the log and changes nothing. Older messages stay on the relay until
/// someone scrolls to them (`older_messages`).
async fn first_sync(app: &Arc<App>, url: &str, token: &str, machine_file: &crate::keys::MachineFile) -> Result<FirstSync, RelayError> {
    app.bulk_sync.store(true, Ordering::Relaxed);
    let synced = first_sync_quietly(app, url, token, machine_file).await;
    app.bulk_sync.store(false, Ordering::Relaxed);
    // A held first sync keeps `last_seq` at 0 but did apply the roster, credentials and policy
    // before the envelope it stopped at: those are written and shown now, not lost to a restart.
    if matches!(synced, Ok(FirstSync::Done | FirstSync::Held)) {
        app.save_state_now();
        app.emit(Event::Snapshot(app.snapshot()));
    }
    app.turns_changed();
    synced
}

#[derive(PartialEq)]
enum FirstSync {
    Done,
    /// The relay's log is empty.
    NothingThere,
    /// The relay cannot page a chat: the caller replays the log.
    CannotPage,
    /// An update holds new work back and the log has an envelope that starts some.
    Held,
}

async fn first_sync_quietly(app: &Arc<App>, url: &str, token: &str, machine_file: &crate::keys::MachineFile) -> Result<FirstSync, RelayError> {
    // The relay keeps the latest roster alone, so the chats have their names and bots before
    // their messages land.
    let (mut since, mut head) = (0, None);
    loop {
        let (blobs, seq) = app.relay.list_blobs(url, token, since, NOT_CHAT_KINDS).await?;
        let head = *head.get_or_insert(seq);
        let Some(last) = blobs.last().map(|blob| blob.seq) else {
            if head == 0 {
                return Ok(FirstSync::NothingThere);
            }
            break;
        };
        for blob in &blobs {
            // While an update holds new work back, the first sync stops at an envelope that
            // starts work; `last_seq` stays 0, so the next pull starts it over (`update_control`).
            let admission = if starts_work(machine_file, blob) {
                let Some(admission) = app.update.try_admit() else { return Ok(FirstSync::Held) };
                Some(admission)
            } else {
                None
            };
            apply_incoming_blob(app, machine_file, blob, true)?;
            drop(admission);
            if blob.kind == "roster" {
                let mut state = app.state.lock().unwrap();
                state.roster_slot_seq = state.roster_slot_seq.max(blob.seq);
            }
        }
        since = last;
    }
    // The credentials are here, so onboarding can tell whether the account has a provider
    // without waiting for every chat.
    mark_account_pulled(app, machine_file);
    let chats: Vec<String> = app.state.lock().unwrap().chats.iter().map(|chat| chat.meta.id.clone()).collect();
    for chat_id in chats {
        let page = app.relay.group_page(url, token, &crate::model::relay_name(&chat_id), None, FIRST_SYNC_MESSAGES).await;
        let (slots, has_more) = match page {
            Ok(page) => page,
            Err(error) if matches!(error.status, Some(404 | 405)) => return Ok(FirstSync::CannotPage),
            Err(error) => return Err(error),
        };
        for blob in slots.iter().flat_map(|slot| &slot.blobs) {
            apply_blob(app, machine_file, blob);
        }
        let before = slots.first().map(|slot| slot.place).filter(|_| has_more);
        if let Err(error) = app.store.set_history_before(&chat_id, before) {
            tracing::error!(%error, %chat_id, "recording where the chat begins");
        }
        // A page ends early at its byte budget; a bot's turn wants the whole count.
        let mut here = slots.len();
        while here < FIRST_SYNC_MESSAGES && app.history_is_partial(&chat_id) {
            here += older_messages_from(app, url, token, machine_file, &chat_id).await?.max(1);
        }
    }
    app.state.lock().unwrap().last_seq = head.unwrap_or(0);
    Ok(FirstSync::Done)
}

/// One chat at a time reads backwards, so two askers do not both take the same page.
static READING_BACK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Fetches the page of messages before the ones this Device has of a chat. Returns how many
/// landed; 0 when the chat is here whole.
pub async fn older_messages(app: &Arc<App>, chat_id: &str) -> Result<usize, String> {
    let url = app.relay_url().ok_or("No relay configured")?;
    let machine_file = app.machine_file().ok_or("This Device is not paired")?;
    let machine = machine_file.machine().map_err(|e| e.to_string())?;
    let token = token_or_register(app, &url, &machine).await.map_err(|e| e.message)?;
    older_messages_from(app, &url, &token, &machine_file, chat_id).await.map_err(|e| e.message)
}

async fn older_messages_from(app: &Arc<App>, url: &str, token: &str, machine_file: &crate::keys::MachineFile, chat_id: &str) -> Result<usize, RelayError> {
    let _reading = READING_BACK.lock().await;
    let local = |error: anyhow::Error| RelayError { status: None, message: error.to_string() };
    let Some(before) = app.store.history_before(chat_id).map_err(local)? else { return Ok(0) };
    let (slots, has_more) = app.relay.group_page(url, token, &crate::model::relay_name(chat_id), Some(before), OLDER_PAGE).await?;
    let dek = machine_file.dek().map_err(local)?;
    // A slot's last blob is the message as it stands. A removal or a read mark is nothing to
    // show, and a read mark from back then must not clear what is unread now.
    let newest_first: Vec<Message> = slots
        .iter()
        .rev()
        .filter_map(|slot| slot.blobs.last())
        .filter_map(|blob| crate::crypto::decrypt_json::<ChatBlob>(&dek, "chat", &unb64(&blob.ciphertext).ok()?).ok())
        .filter_map(|op| match op {
            ChatBlob::Upsert { message } if message.chat_id == chat_id => Some(message),
            _ => None,
        })
        .collect();
    app.store.insert_older(&newest_first).map_err(local)?;
    let before = slots.first().map(|slot| slot.place).filter(|_| has_more);
    app.store.set_history_before(chat_id, before).map_err(local)?;
    Ok(newest_first.len())
}

/// Pulls the log from `last_seq` until a page comes back empty.
async fn pull_blobs(app: &Arc<App>, url: &str, token: &str, machine_file: &crate::keys::MachineFile) -> Result<(), RelayError> {
    if app.state.lock().unwrap().last_seq == 0 {
        match first_sync(app, url, token, machine_file).await? {
            FirstSync::CannotPage => {
                // A relay that cannot page a chat: replay its log. It keeps the latest roster,
                // which in a replay comes after the messages, so that is taken first as a preview.
                let (blobs, _head) = app.relay.list_blobs(url, token, 0, "roster,policy,machine").await?;
                for blob in blobs {
                    apply_incoming_blob(app, machine_file, &blob, false)?;
                }
            }
            // The held first sync still lets Pause and a running job's Stop through.
            FirstSync::Held => return pull_controls(app, url, token, machine_file).await,
            FirstSync::Done | FirstSync::NothingThere => {}
        }
    }
    loop {
        let since = app.state.lock().unwrap().last_seq;
        let (blobs, _head) = app.relay.list_blobs(url, token, since, POLL_KINDS).await?;
        for blob in &blobs {
            if blob.kind == "roster" {
                // Validation happens again under roster_edit at application time.
                // This pass prevents applying earlier blobs in a conflicting page.
                if let Some(item) = app.store.queued_roster().map_err(local_relay_error)? {
                    validate_roster_blob(app, machine_file, &item, blob)?;
                }
            }
        }
        if blobs.is_empty() {
            // Caught up, the replay of a relay that cannot page a chat included.
            mark_account_pulled(app, machine_file);
            app.state.lock().unwrap().caught_up = true;
            if settle_unknown_machines(app) {
                app.emit(app.roster_summary());
            }
            return Ok(());
        }
        // A page this long is a backlog (a fresh pair replays the history): apply it quietly
        // and tell the app once, instead of one event and one state write per message.
        let bulk = blobs.len() >= BULK_BLOBS;
        app.bulk_sync.store(bulk, Ordering::Relaxed);
        // A backlog's messages reach the app with its snapshot, and a turn's end must come
        // after the messages it ended with, so its results wait for that snapshot.
        let mut results = Vec::new();
        let mut held = false;
        for blob in blobs {
            let seq = blob.seq;
            let is_roster = blob.kind == "roster";
            // An envelope that starts work here waits on the relay while an update holds new
            // work back: the cursor stops before it, so it and everything after it come again
            // once the lease ends (`update_control`).
            let admission = if starts_work(machine_file, &blob) {
                let Some(admission) = app.update.try_admit() else {
                    held = true;
                    break;
                };
                Some(admission)
            } else {
                None
            };
            if bulk && blob.kind == "job_result" {
                results.push(blob);
            } else {
                apply_incoming_blob(app, machine_file, &blob, true)?;
            }
            drop(admission);
            let mut state = app.state.lock().unwrap();
            if is_roster {
                state.roster_slot_seq = state.roster_slot_seq.max(seq);
            }
            state.last_seq = state.last_seq.max(seq);
        }
        app.bulk_sync.store(false, Ordering::Relaxed);
        app.save_state_now();
        if bulk {
            app.emit(Event::Snapshot(app.snapshot()));
            app.turns_changed();
        }
        for blob in results {
            apply_blob(app, machine_file, &blob);
        }
        // Releasing or expiring the lease wakes the session loop, which pulls from here again.
        if held {
            return pull_controls(app, url, token, machine_file).await;
        }
    }
}

/// While the cursor waits before an envelope an update holds back, running work still hears
/// Pause, Stop, requests answering or controlling its cards and commands, and the `job_result`
/// or `response` a wait here is pending on. Policy replays by version; consumed envelopes replay
/// as no-ops. A cancellation for a job not running here, or a completion nothing here waits on,
/// stays unrecorded on the relay for the main pull to take in order. The cursor stays put.
async fn pull_controls(app: &Arc<App>, url: &str, token: &str, machine_file: &crate::keys::MachineFile) -> Result<(), RelayError> {
    let Ok(machine) = machine_file.machine() else { return Ok(()) };
    let mut since = app.state.lock().unwrap().last_seq;
    loop {
        let (blobs, _) = app.relay.list_blobs(url, token, since, "policy,job_cancel,job_result,request,response").await?;
        let Some(last) = blobs.last().map(|blob| blob.seq) else { return Ok(()) };
        for blob in &blobs {
            let applies = match blob.kind.as_str() {
                "policy" => true,
                "job_cancel" => unb64(&blob.ciphertext)
                    .ok()
                    .and_then(|ciphertext| crate::crypto::unseal_json::<JobCancel>(&machine.box_secret, &ciphertext).ok())
                    .is_some_and(|cancel| app.running_jobs.lock().unwrap().contains_key(&cancel.job_id)),
                "request" => unb64(&blob.ciphertext)
                    .ok()
                    .and_then(|ciphertext| crate::crypto::unseal_json::<Request>(&machine.box_secret, &ciphertext).ok())
                    .is_some_and(|request| continues_work(&request.verb)),
                "job_result" => unb64(&blob.ciphertext)
                    .ok()
                    .and_then(|ciphertext| crate::crypto::unseal_json::<JobResult>(&machine.box_secret, &ciphertext).ok())
                    .is_some_and(|result| app.pending_results.lock().unwrap().contains_key(&result.job_id)),
                "response" => unb64(&blob.ciphertext)
                    .ok()
                    .and_then(|ciphertext| crate::crypto::unseal_json::<Response>(&machine.box_secret, &ciphertext).ok())
                    .is_some_and(|response| app.pending_responses.lock().unwrap().contains_key(&response.request_id)),
                _ => false,
            };
            if applies {
                apply_blob(app, machine_file, blob);
            }
        }
        since = last;
    }
}

/// Whether an envelope starts work on this Runner: a job, or a request other than an answer to
/// work already running here (a permission card, a command's input or Stop), which running work
/// may be waiting on.
fn starts_work(machine_file: &crate::keys::MachineFile, blob: &BlobIn) -> bool {
    match blob.kind.as_str() {
        "job" => true,
        "request" => {
            let (Ok(machine), Ok(ciphertext)) = (machine_file.machine(), unb64(&blob.ciphertext)) else { return true };
            crate::crypto::unseal_json::<Request>(&machine.box_secret, &ciphertext)
                .map_or(true, |request| !continues_work(&request.verb))
        }
        _ => false,
    }
}

fn continues_work(verb: &str) -> bool {
    matches!(verb, "permission.answer" | "bash.stdin" | "bash.stop" | "bash.background" | "chats.send_now")
}

/// How long `sync.account` waits for the first pull of an account this Device just joined.
pub const ACCOUNT_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The roster, Devices, and credentials of the account `machine_file` belongs to are here.
fn mark_account_pulled(app: &App, machine_file: &crate::keys::MachineFile) {
    let Ok(machine) = machine_file.machine() else { return };
    let id = machine.pubkey();
    app.account_pulled.send_if_modified(|pulled| {
        let changed = pulled.as_deref() != Some(id.as_str());
        if changed {
            *pulled = Some(id);
        }
        changed
    });
}

/// Waits, for at most `limit`, until the sync loop has pulled the account this Device holds:
/// a Device that pairs or restores gets the account's credentials that way, so onboarding asks
/// before it offers to connect a provider. False when the limit passed first, or this Device
/// holds no account.
pub async fn wait_for_account(app: &App, limit: std::time::Duration) -> bool {
    let Some(this) = app.this_device_id() else { return false };
    let mut pulled = app.account_pulled.subscribe();
    let landed = tokio::time::timeout(limit, pulled.wait_for(|machine| machine.as_deref() == Some(this.as_str()))).await;
    landed.is_ok_and(|landed| landed.is_ok())
}

/// A bearer for this machine. When the relay does not know the machine (a relay other than
/// the one that attested it, or a reset one) and this Device holds the identity, it attests
/// itself again and retries.
pub async fn token_or_register(app: &Arc<App>, url: &str, machine: &crate::keys::Machine) -> Result<String, RelayError> {
    match app.relay.token(url, machine).await {
        Err(error) if error.is_unknown_machine() && app.is_identity_device() => {
            tracing::info!(url, "relay does not know this machine; attesting it again");
            ensure_registered(app, url).await?;
            app.relay.token(url, machine).await
        }
        result => result,
    }
}

/// Registers this machine with the relay. Only an identity device can sign that.
pub async fn ensure_registered(app: &Arc<App>, url: &str) -> Result<(), RelayError> {
    let identity = app
        .identity
        .lock()
        .unwrap()
        .clone()
        .and_then(|file| file.identity().ok())
        .ok_or_else(|| RelayError { status: None, message: "this Device is not registered and holds no identity key".into() })?;
    let machine = app.machine_file().and_then(|m| m.machine().ok()).ok_or_else(|| RelayError { status: None, message: "no machine".into() })?;
    app.relay.register(url, &identity, &machine.pubkey(), &machine.box_pubkey()).await?;
    let again = app.machine.lock().unwrap().as_mut().is_some_and(|file| std::mem::replace(&mut file.registered, true));
    // A relay that had to be told about this machine has none of its blobs either.
    {
        let mut state = app.state.lock().unwrap();
        state.machine_blob_hash = None;
        state.credentials_uploaded = false;
        if again {
            // It numbers its log from one, so the place held in the old log means nothing.
            state.last_seq = 0;
            state.roster_slot_seq = 0;
        }
    }
    // A relay that knew this machine before and lost the account (a reset, or the identity
    // dropped for inactivity) gets back what a Device pairing or restoring needs: the DEK
    // sealed to the content key, the roster, and the chats' messages as this Device has them.
    if again {
        if let Some(dek) = app.dek() {
            match crate::crypto::seal(&identity.content_pubkey(), &dek) {
                Ok(sealed) => {
                    app.push_blob("key", None, sealed);
                }
                Err(error) => tracing::error!(%error, "sealing the account key"),
            }
        }
        app.push_roster();
        app.push_history();
    }
    app.save_machine().map_err(|e| RelayError { status: None, message: e.to_string() })?;
    Ok(())
}

async fn drain_outbox(app: &Arc<App>, url: &str, token: &str) -> Result<(), RelayError> {
    loop {
        let Some(item) = app
            .store
            .first_outbox()
            .map_err(|error| RelayError { status: None, message: error.to_string() })?
        else {
            return Ok(());
        };
        let (id, kind) = (item.id.clone(), item.kind.clone());
        let accepted_roster = if kind == "roster" {
            app.dek().and_then(|dek| crate::crypto::decrypt_json::<RosterBlob>(&dek, "roster", &item.ciphertext).ok())
        } else { None };
        let expected_slot_seq = app.state.lock().unwrap().roster_slot_seq;
        if let Some(roster) = &accepted_roster {
            app.store.record_submitted_roster(expected_slot_seq, roster).map_err(local_relay_error)?;
        }
        match app.relay.put_blob(url, token, item, expected_slot_seq).await {
            Ok(seq) if kind == "roster" => {
                let _edit = app.roster_edit.lock().unwrap();
                let published_chat_ids = accepted_roster.as_ref().map(|roster| {
                    let pending = app.store.pending_chat_creates().map_err(local_relay_error)?;
                    Ok::<Vec<String>, RelayError>(roster.chats.iter().filter(|chat| pending.contains(&chat.id))
                        .map(|chat| chat.id.clone()).collect())
                }).transpose()?.unwrap_or_default();
                let snapshot = {
                    let mut state = app.state.lock().unwrap();
                    state.roster_slot_seq = state.roster_slot_seq.max(seq);
                    state.clone()
                };
                if let Some(roster) = accepted_roster {
                    let newer = app.store.queued_roster().map_err(local_relay_error)?
                        .filter(|current| current.id != id);
                    let baseline = if newer.is_some() { app.store.roster_baseline("queued").map_err(local_relay_error)? } else { None };
                    app.store.observe_roster(seq, &roster).map_err(local_relay_error)?;
                    if let (Some(current), Some((_, base))) = (newer, baseline) {
                        let dek = app.dek().ok_or_else(|| roster_conflict("account key unavailable"))?;
                        let local: RosterBlob = crate::crypto::decrypt_json(&dek, "roster", &current.ciphertext).map_err(local_relay_error)?;
                        let base = accepted_roster_baseline(base, &roster);
                        let merged = merge_rosters(roster, &base, &local);
                        validate_merged_groups(&merged)?;
                        let rebased = OutboxItem { id: uuid::Uuid::new_v4().to_string(), kind: "roster".into(), recipient: None,
                            ciphertext: crate::crypto::encrypt_json(&dek, "roster", &merged).map_err(local_relay_error)?,
                            slot: Some(Slot::latest("roster")), group: None };
                        app.store.rebase_queued_roster_with_state(&current.id, &rebased, &snapshot).map_err(local_relay_error)?;
                    }
                }
                app.store.remove_outbox_roster_with_state(&id, &snapshot, &published_chat_ids).map_err(local_relay_error)?;
                continue;
            }
            Ok(_) => {}
            // Over the relay's budget for this identity, as a history uploaded again is: the
            // same blob goes after a second.
            Err(error) if error.is_rate_limited() => {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
            // Keep the exact queued roster and its creation markers. Next sync round pulls
            // the winner, rebases the queued edit, then retries after the outer backoff.
            Err(error) if kind == "roster" && error.status == Some(409) => return Err(error),
            Err(error) if kind == "policy" => return Err(error),
            Err(error) if error.is_client_error() && !error.is_unauthorized() => {
                tracing::warn!(%error, %kind, "relay rejected blob; dropping");
                if kind == "roster" { return Err(error); }
                if kind == "credentials" {
                    app.state.lock().unwrap().credentials_uploaded = false;
                }
            }
            Err(error) => return Err(error),
        }
        let snapshot = app.state.lock().unwrap().clone();
        app.store.remove_outbox_with_state(&id, &snapshot).map_err(local_relay_error)?;
    }
}

/// Tells the relay to drop the blobs of the chats deleted here. A relay that refuses (one
/// without groups) is not asked again; one that is away is asked on the next cycle.
async fn drain_group_deletes(app: &Arc<App>, url: &str, token: &str) -> Result<(), RelayError> {
    loop {
        let Some(group) = app.state.lock().unwrap().group_deletes.first().cloned() else { return Ok(()) };
        match app.relay.delete_group(url, token, &group).await {
            Ok(()) => {}
            Err(error) if error.is_client_error() && !error.is_unauthorized() && !error.is_unpaired() => {
                tracing::warn!(%error, "relay refused a group delete; dropping");
            }
            Err(error) => return Err(error),
        }
        app.state.lock().unwrap().group_deletes.retain(|g| g != &group);
        app.save_state();
    }
}

/// Tells the relay to drop the `file` blobs of the avatars dropped here. One the relay does
/// not have (never uploaded, or deleted by another Device) is done; a relay that is away is
/// asked on the next cycle.
async fn drain_blob_deletes(app: &Arc<App>, url: &str, token: &str) -> Result<(), RelayError> {
    loop {
        let Some(id) = app.state.lock().unwrap().blob_deletes.first().cloned() else { return Ok(()) };
        match app.relay.delete_blob(url, token, &id).await {
            Ok(()) => {}
            Err(error) if error.is_client_error() && !error.is_unauthorized() && !error.is_unpaired() => {
                tracing::debug!(%error, id, "relay had no such avatar blob; dropping");
            }
            Err(error) => return Err(error),
        }
        app.state.lock().unwrap().blob_deletes.retain(|queued| queued != &id);
        app.save_state();
    }
}

/// The relay's machine list is the list of paired Devices: presence comes from it, and a
/// Device it no longer lists was unpaired, so it leaves the roster here too.
async fn refresh_presence(app: &Arc<App>, url: &str, token: &str) -> Result<(), RelayError> {
    let (machines, relay_now) = app.relay.machines(url, token).await?;
    // When the relay attested each machine, on this Device's clock.
    let skew = crate::config::now_unix() - relay_now;
    let this_id = app.this_device_id();
    let (changed, pruned) = {
        let mut state = app.state.lock().unwrap();
        let before: Vec<bool> = state.devices.iter().map(|d| online(&state, &d.id)).collect();
        state.listed_machines = machines.iter().map(|m| (m.machine_pubkey.clone(), m.created_at + skew)).collect();
        state.device_online = machines.iter().filter(|m| m.online).map(|m| m.machine_pubkey.clone()).collect();
        for machine in &machines {
            state.device_seen.insert(machine.machine_pubkey.clone(), machine.last_seen);
            if let Some(device) = state.devices.iter_mut().find(|d| d.id == machine.machine_pubkey) {
                if device.box_pubkey.is_empty() {
                    device.box_pubkey = machine.box_pubkey.clone();
                }
            }
        }
        let count = state.devices.len();
        state.devices.retain(|d| Some(&d.id) == this_id.as_ref() || machines.iter().any(|m| m.machine_pubkey == d.id));
        let pruned = state.devices.len() != count;
        if pruned {
            state.device_seen.retain(|id, _| Some(id) == this_id.as_ref() || machines.iter().any(|m| &m.machine_pubkey == id));
        }
        state.turns_online = state.device_online.clone();
        let after: Vec<bool> = state.devices.iter().map(|d| online(&state, &d.id)).collect();
        (before != after, pruned)
    };
    if pruned {
        app.save_state();
    }
    let unknown = settle_unknown_machines(app);
    if changed || pruned || unknown {
        app.emit(app.roster_summary());
    }
    // A Device that is not online lists nothing: a Runner that stopped mid-turn never shows
    // that turn again, and one that only lost the relay lists it again when it connects.
    let offline: Vec<String> = {
        let mut state = app.state.lock().unwrap();
        let offline: Vec<String> = state.device_turns.keys().filter(|id| !state.turns_online.contains(*id)).cloned().collect();
        for id in &offline {
            state.device_turns.remove(id);
        }
        offline
    };
    for id in offline {
        if let Err(error) = app.store.set_device_turns(&id, &[]) {
            tracing::error!(%error, "dropping another Device's turns");
        }
    }
    Ok(())
}

/// How long the relay may list a machine before the Device list shows it as unknown when no
/// `machine` blob came from it: a Device that pairs sends its blob within seconds.
const UNKNOWN_AFTER: i64 = 10 * 60;

/// Works out which machines the relay lists that never said what they are: no `machine` blob
/// in what this Device pulled, ten minutes after the relay attested them. The Device list shows
/// those as unknown, so a machine nobody recognizes can be unpaired. Only once this session's
/// pull has caught up, so a Device still reading the log does not take the Devices whose blobs
/// it has yet to read for unknown ones; until then the last answer stands. A machine still
/// inside its ten minutes is looked at again when they end. Returns whether the set changed,
/// for the caller to tell the app.
fn settle_unknown_machines(app: &Arc<App>) -> bool {
    let this_id = app.this_device_id();
    let now = crate::config::now_unix();
    let (changed, next) = {
        let mut state = app.state.lock().unwrap();
        if !state.caught_up {
            return false;
        }
        let mut next: Option<i64> = None;
        let mut unknown = std::collections::BTreeSet::new();
        for (id, attested) in &state.listed_machines {
            if Some(id) == this_id.as_ref() || state.devices.iter().any(|d| &d.id == id) {
                continue;
            }
            let due = attested + UNKNOWN_AFTER;
            if due <= now {
                unknown.insert(id.clone());
            } else {
                next = Some(next.map_or(due, |next: i64| next.min(due)));
            }
        }
        let changed = state.unknown_machines != unknown;
        state.unknown_machines = unknown;
        (changed, next)
    };
    if let Some(due) = next {
        let app = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs((due - now).max(1) as u64)).await;
            if settle_unknown_machines(&app) {
                app.emit(app.roster_summary());
            }
        });
    }
    changed
}

/// Unpairs another Device: the relay drops its key, and it leaves this roster now rather
/// than on the next presence refresh. A machine the relay already forgot still leaves.
pub async fn unpair_device(app: &Arc<App>, id: &str) -> Result<(), String> {
    let url = app.relay_url().ok_or("Set a relay URL first.")?;
    let machine = app.machine_file().and_then(|m| m.machine().ok()).ok_or("No identity on this Device")?;
    let token = token_or_register(app, &url, &machine).await.map_err(|e| e.to_string())?;
    match app.relay.revoke_machine(&url, &token, id).await {
        Ok(()) => {}
        Err(error) if error.is_unknown_machine() => {}
        Err(error) => return Err(error.to_string()),
    }
    {
        let mut state = app.state.lock().unwrap();
        state.devices.retain(|d| d.id != id);
        state.device_seen.remove(id);
        state.device_online.remove(id);
        state.turns_online.remove(id);
        state.listed_machines.remove(id);
        state.unknown_machines.remove(id);
    }
    app.save_state();
    app.emit(app.roster_summary());
    app.set_device_turns(id, Vec::new());
    app.turns_changed();
    Ok(())
}

/// Before this Device forgets the identity, it asks the relay to drop its key, so the other
/// Devices see it leave instead of an offline ghost. Best effort: the relay may be away.
pub async fn revoke_self(app: &Arc<App>) {
    let (Some(url), Some(machine)) = (app.relay_url(), app.machine_file().and_then(|m| m.machine().ok())) else { return };
    let revoke = async {
        let token = app.relay.token(&url, &machine).await?;
        app.relay.revoke_machine(&url, &token, &machine.pubkey()).await
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), revoke).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(%error, "revoking this machine on the relay"),
        Err(_) => tracing::warn!("revoking this machine on the relay timed out"),
    }
}

/// Deletes the account on the relay: every Device, this one among them, is unpaired by it.
/// Unlike `revoke_self` this has to land, or the relay keeps the data the user asked gone.
pub async fn delete_identity(app: &Arc<App>) -> Result<(), String> {
    let Some(url) = app.relay_url() else { return Ok(()) };
    let machine = app.machine_file().and_then(|m| m.machine().ok()).ok_or("This Device has no identity")?;
    let token = token_or_register(app, &url, &machine).await.map_err(|e| e.message)?;
    match app.relay.delete_identity(&url, &token).await {
        Ok(()) => Ok(()),
        // The relay already dropped this Device, with the account or without it.
        Err(error) if error.is_unpaired() => Ok(()),
        Err(error) => Err(error.message),
    }
}

fn online(state: &crate::app::State, id: &str) -> bool {
    state.device_online.contains(id)
}

pub async fn delete_remote_blob(app: &Arc<App>, id: &str) {
    let (Some(url), Some(machine)) = (app.relay_url(), app.machine_file().and_then(|m| m.machine().ok())) else { return };
    if let Ok(token) = app.relay.token(&url, &machine).await {
        if let Err(error) = app.relay.delete_blob(&url, &token, id).await {
            tracing::debug!(%error, id, "deleting consumed job blob");
        }
    }
}

// MARK: - Applying blobs

fn local_relay_error(error: anyhow::Error) -> RelayError {
    RelayError { status: None, message: error.to_string() }
}

fn apply_incoming_blob(app: &Arc<App>, machine_file: &crate::keys::MachineFile, blob: &BlobIn,
    remember: bool) -> Result<(), RelayError> {
    if blob.kind != "roster" {
        if remember { apply_blob(app, machine_file, blob); }
        else { apply_blob_contents(app, machine_file, blob); }
        return Ok(());
    }
    let _edit = app.roster_edit.lock().unwrap();
    let already_applied = app.state.lock().unwrap().applied_blob_ids.contains(&blob.id);
    if remember && already_applied && app.store.roster_baseline("observed").map_err(local_relay_error)?
        .is_some_and(|(seq, _)| seq >= blob.seq) { return Ok(()); }
    let dek = machine_file.dek().map_err(local_relay_error)?;
    let roster: RosterBlob = crate::crypto::decrypt_json(&dek, "roster", &unb64(&blob.ciphertext).map_err(local_relay_error)?)
        .map_err(local_relay_error)?;
    // Reload queue and markers while edits cannot change them; validate before observing,
    // changing state, or deleting any transcript.
    let queue = app.store.queued_roster().map_err(local_relay_error)?;
    if let Some(item) = &queue { validate_roster_blob(app, machine_file, item, blob)?; }
    else {
        validate_pending_chat_identities(app, &roster)?;
    }
    let markers = app.store.pending_chat_identities().map_err(local_relay_error)?;
    let accepted_ids: Vec<String> = markers.iter().filter_map(|(id, original)| {
        let expected = original.as_ref()?;
        roster.chats.iter().any(|remote| remote.id == *id && remote.kind == expected.kind
            && remote.bot_ids == expected.bot_ids && remote.owner_bot_id == expected.owner_bot_id).then(|| id.clone())
    }).collect();
    let omitted_deleted_ids: Vec<String> = markers.iter().filter(|(id, _)|
        !roster.chats.iter().any(|chat| chat.id == *id)
        && !app.state.lock().unwrap().chats.iter().any(|chat| chat.meta.id == *id))
        .map(|(id, _)| id.clone()).collect();
    let resolved_ids: Vec<String> = accepted_ids.iter().chain(&omitted_deleted_ids).cloned().collect();
    let own_accepted = already_applied;
    let baseline = app.store.roster_baseline("queued").map_err(local_relay_error)?;
    let submitted = if queue.is_some() && baseline.is_some() && !own_accepted {
        app.store.roster_baseline("submitted").map_err(local_relay_error)?
    } else { None };
    // Routines have no chat creation marker. A submitted new routine present in a
    // later slot establishes its creation without claiming unrelated remote additions.
    let routine_creation_accepted = baseline.as_ref().zip(submitted.as_ref()).is_some_and(
        |((_, base), (submitted_seq, sent))| blob.seq > *submitted_seq
            && sent.routines.iter().any(|routine| !base.routines.iter().any(|old| old.id == routine.id)
                && roster.routines.iter().any(|remote| remote.id == routine.id && remote.bot_id == routine.bot_id)));
    let recovering = own_accepted || !accepted_ids.is_empty() || routine_creation_accepted;
    let projection = match (&queue, baseline) {
        (Some(item), Some((_, base))) => {
            let queued: RosterBlob = crate::crypto::decrypt_json(&dek, "roster", &item.ciphertext).map_err(local_relay_error)?;
            let base = if !recovering { base }
                else if own_accepted { accepted_roster_baseline(base, &roster) }
                else {
                    let (_, submitted) = submitted
                        .ok_or_else(|| roster_conflict("accepted creation lacks its original submitted roster"))?;
                    accepted_roster_baseline(base, &submitted)
                };
            let projected = merge_rosters(roster.clone(), &base, &queued);
            validate_merged_groups(&projected)?;
            if recovering {
                let rebased = OutboxItem { id: uuid::Uuid::new_v4().to_string(), kind: "roster".into(), recipient: None,
                    ciphertext: crate::crypto::encrypt_json(&dek, "roster", &projected).map_err(local_relay_error)?,
                    slot: Some(Slot::latest("roster")), group: None };
                app.store.observe_roster(blob.seq, &roster).map_err(local_relay_error)?;
                app.store.rebase_queued_roster_with_state(&item.id, &rebased, &app.state.lock().unwrap().clone()).map_err(local_relay_error)?;
                app.store.acknowledge_chat_creates(&resolved_ids).map_err(local_relay_error)?;
            } else {
                app.store.observe_roster(blob.seq, &roster).map_err(local_relay_error)?;
                app.store.acknowledge_chat_creates(&omitted_deleted_ids).map_err(local_relay_error)?;
            }
            projected
        }
        _ => {
            app.store.observe_roster(blob.seq, &roster).map_err(local_relay_error)?;
            app.store.acknowledge_chat_creates(&resolved_ids).map_err(local_relay_error)?;
            roster
        }
    };
    apply_roster(app, projection);
    if remember { remember_applied(&mut app.state.lock().unwrap(), &blob.id); }
    Ok(())
}

pub fn apply_blob(app: &Arc<App>, machine_file: &crate::keys::MachineFile, blob: &BlobIn) {
    if blob.kind == "roster" {
        if let Err(error) = apply_incoming_blob(app, machine_file, blob, true) {
            tracing::warn!(%error, "applying roster blob");
        }
        return;
    }
    let already = {
        let mut state = app.state.lock().unwrap();
        if state.applied_blob_ids.iter().any(|id| id == &blob.id) {
            true
        } else {
            remember_applied(&mut state, &blob.id);
            false
        }
    };
    if !already { apply_blob_contents(app, machine_file, blob); }
}

/// Applies a blob whether or not it was applied before, and leaves no record of it.
fn apply_blob_contents(app: &Arc<App>, machine_file: &crate::keys::MachineFile, blob: &BlobIn) {
    let Ok(ciphertext) = unb64(&blob.ciphertext) else { return };
    let Ok(dek) = machine_file.dek() else { return };

    match blob.kind.as_str() {
        "roster" => {
            // Roster application uses apply_incoming_blob to validate under roster_edit.
        },
        "policy" => match crate::crypto::decrypt_json::<PolicyBlob>(&dek, "policy", &ciphertext) {
            Ok(policy) => apply_policy(app, policy),
            Err(error) => tracing::warn!(%error, "policy blob"),
        },
        "chat" => match crate::crypto::decrypt_json::<ChatBlob>(&dek, "chat", &ciphertext) {
            Ok(op) => apply_chat_op(app, op),
            Err(error) => tracing::warn!(%error, "chat blob"),
        },
        "machine" => match crate::crypto::decrypt_json::<MachineBlob>(&dek, "machine", &ciphertext) {
            Ok(MachineBlob { device, turns }) => {
                if app.this_device_id().as_deref() == Some(device.id.as_str()) {
                    return;
                }
                // Shown only while the relay lists the Device online, so a key it no longer
                // lists shows nothing.
                app.set_device_turns(&device.id, turns);
                let mut state = app.state.lock().unwrap();
                // A key the last presence refresh did not list is either a Device that just
                // paired or one unpaired since, whose old blob must not bring it back. It
                // lands quietly, and the refresh at the end of the cycle confirms or prunes it.
                if !state.device_seen.contains_key(&device.id) {
                    upsert_device(&mut state.devices, device);
                    app.presence_stale.store(true, Ordering::Relaxed);
                    return;
                }
                // A blob that only lists other turns is stamped anew but changes nothing the
                // roster shows.
                let restamped = state.devices.iter().any(|known| *known == Device { updated_at: known.updated_at, ..device.clone() });
                let changed = !restamped && upsert_device(&mut state.devices, device);
                drop(state);
                if changed {
                    app.save_state();
                    app.emit(app.roster_summary());
                }
            }
            Err(error) => tracing::warn!(%error, "machine blob"),
        },
        "credentials" => match crate::crypto::decrypt_json::<crate::credentials::Credentials>(&dek, "credentials", &ciphertext) {
            Ok(credentials) => app.apply_credentials(&credentials),
            Err(error) => tracing::warn!(%error, "credentials blob"),
        },
        "job" => {
            let Ok(machine) = machine_file.machine() else { return };
            match crate::crypto::unseal_json::<Job>(&machine.box_secret, &ciphertext) {
                Ok(job) => {
                    // The pull that applies this holds its admission (`starts_work`).
                    crate::runtime::spawn_local_job(app.clone(), job, Some(blob.id.clone()), app.update.hold());
                }
                Err(error) => tracing::warn!(%error, "job envelope"),
            }
        }
        "job_cancel" => {
            let Ok(machine) = machine_file.machine() else { return };
            match crate::crypto::unseal_json::<JobCancel>(&machine.box_secret, &ciphertext) {
                Ok(cancel) => {
                    app.cancel_job(&cancel.job_id);
                    let app = app.clone();
                    let blob_id = blob.id.clone();
                    tokio::spawn(async move { delete_remote_blob(&app, &blob_id).await });
                }
                Err(error) => tracing::warn!(%error, "job cancellation envelope"),
            }
        }
        "job_result" => {
            let Ok(machine) = machine_file.machine() else { return };
            match crate::crypto::unseal_json::<JobResult>(&machine.box_secret, &ciphertext) {
                Ok(result) => {
                    crate::runtime::deliver_job_result(app, result);
                    let app = app.clone();
                    let blob_id = blob.id.clone();
                    tokio::spawn(async move { delete_remote_blob(&app, &blob_id).await });
                }
                Err(error) => tracing::warn!(%error, "job result envelope"),
            }
        }
        "request" => {
            let Ok(machine) = machine_file.machine() else { return };
            match crate::crypto::unseal_json::<Request>(&machine.box_secret, &ciphertext) {
                Ok(request) => crate::requests::serve(app.clone(), request, blob.id.clone(), app.update.hold()),
                Err(error) => tracing::warn!(%error, "request envelope"),
            }
        }
        "response" => {
            let Ok(machine) = machine_file.machine() else { return };
            match crate::crypto::unseal_json::<Response>(&machine.box_secret, &ciphertext) {
                Ok(response) => crate::requests::deliver(app.clone(), response, blob.id.clone()),
                Err(error) => tracing::warn!(%error, "response envelope"),
            }
        }
        _ => {}
    }
}

fn apply_policy(app: &Arc<App>, policy: PolicyBlob) {
    let _edit = app.roster_edit.lock().unwrap();
    if policy.version.counter == 0 || policy.version.device_id.is_empty() { return; }
    let mut roster_changed = false;
    let mut removed = Vec::new();
    let mut removed_bots = Vec::new();
    let mut newly_paused = false;
    {
        let mut state = app.state.lock().unwrap();
        state.policy_clock = state.policy_clock.max(policy.version.counter);
        if let Some(paused) = policy.paused {
            if policy.version > state.pause_version {
                newly_paused = paused && !state.paused;
                state.paused = paused;
                state.pause_version = policy.version.clone();
                roster_changed = true;
            }
        } else if let Some(id) = policy.bot_id {
            if policy.removed {
                if policy.version > *state.deleted_bot_versions.get(&id).unwrap_or(&PolicyVersion::default()) {
                    state.deleted_bot_versions.insert(id.clone(), policy.version);
                    state.capability_versions.remove(&id);
                    state.policy_capabilities.remove(&id);
                    removed_bots.push(id.clone());
                    state.bots.retain(|bot| bot.id != id);
                    state.routines.retain(|routine| routine.bot_id != id);
                    for chat in &mut state.chats {
                        if chat.meta.bot_ids.iter().any(|member| member == &id) {
                            if !chat.meta.is_group() { removed.push(chat.meta.id.clone()); continue; }
                            chat.meta.bot_ids.retain(|member| member != &id);
                            if chat.meta.owner_bot_id.as_deref() == Some(id.as_str()) { chat.meta.owner_bot_id = chat.meta.bot_ids.first().cloned(); }
                            if chat.meta.bot_ids.is_empty() { removed.push(chat.meta.id.clone()); }
                        }
                    }
                    state.chats.retain(|chat| !removed.contains(&chat.meta.id));
                    roster_changed = true;
                }
            } else if let Some(capabilities) = policy.capabilities {
                if capabilities.validate().is_ok() && policy.version > *state.capability_versions.get(&id).unwrap_or(&PolicyVersion::default()) && !state.deleted_bot_versions.contains_key(&id) {
                    state.capability_versions.insert(id.clone(), policy.version);
                    state.policy_capabilities.insert(id.clone(), capabilities.clone());
                    if let Some(bot) = state.bots.iter_mut().find(|bot| bot.id == id) { bot.capabilities = capabilities; }
                    roster_changed = true;
                }
            }
        }
    }
    if newly_paused { app.stop_for_pause(); }
    if !removed.is_empty() {
        let snapshot = app.state.lock().unwrap().clone();
        if let Err(error) = app.store.save_state_deleting_chats(&snapshot, &removed) { tracing::error!(%error, "saving policy removal"); }
        for chat_id in removed { app.cancel_chat(&chat_id); app.emit(Event::ChatRemoved { chat_id }); }
    }
    crate::runtime::cancel_removed_bots(app, &removed_bots);
    if roster_changed {
        // A policy event changes the projection, not the queued offline edit. Rebase that
        // ciphertext with current policy rather than replacing it with the projection.
        let upload = match app.store.queued_roster() {
            Ok(item) => item.is_none(),
            Err(error) => { tracing::error!(%error, "reading queued roster for policy"); false }
        };
        app.roster_changed(upload);
    }
}

fn roster_conflict(reason: &str) -> RelayError {
    RelayError { status: None, message: format!("Roster conflict: {reason}. Local changes remain queued; reconcile the conflicting chat on a paired Device before retrying") }
}

async fn preview_roster_conflict(app: &Arc<App>, url: &str, token: &str,
    machine_file: &crate::keys::MachineFile, _item: &OutboxItem) -> Result<(), RelayError> {
    let (blobs, _) = app.relay.list_blobs(url, token, 0, "roster").await?;
    let Some(item) = app.store.queued_roster().map_err(|error| RelayError { status: None, message: error.to_string() })? else { return Ok(()); };
    if let Some(blob) = blobs.last() { validate_roster_blob(app, machine_file, &item, blob)?; }
    else if legacy_roster_conflict(app.store.roster_baseline("queued").map_err(|error| RelayError { status: None, message: error.to_string() })?.as_ref().map(|(seq, _)| *seq),
        app.state.lock().unwrap().roster_slot_seq, 0) {
        return Err(roster_conflict("queued roster lacks its original baseline and remote roster changed"));
    }
    Ok(())
}

fn validate_roster_blob(app: &Arc<App>, machine_file: &crate::keys::MachineFile,
    item: &OutboxItem, remote_blob: &BlobIn) -> Result<(), RelayError> {
    let local = |error: anyhow::Error| RelayError { status: None, message: error.to_string() };
    let baseline = app.store.roster_baseline("queued").map_err(local)?;
    let queued_seq = app.state.lock().unwrap().roster_slot_seq;
    if legacy_roster_conflict(baseline.as_ref().map(|(seq, _)| *seq), queued_seq, remote_blob.seq) {
        return Err(roster_conflict("queued roster lacks its original baseline and remote roster changed"));
    }
    let dek = machine_file.dek().map_err(local)?;
    let remote: RosterBlob = crate::crypto::decrypt_json(&dek, "roster", &unb64(&remote_blob.ciphertext).map_err(local)?)
        .map_err(local)?;
    validate_pending_chat_identities(app, &remote)?;
    if let Some((_, base)) = baseline {
        let queued: RosterBlob = crate::crypto::decrypt_json(&dek, "roster", &item.ciphertext).map_err(local)?;
        validate_merged_groups(&merge_rosters(remote, &base, &queued))?;
    }
    Ok(())
}

// A locally created chat owns its id until its roster lands. Merging by id would otherwise
// replace its DM (and possibly strand its bot) with a different remote chat.

fn validate_pending_chat_identities(app: &Arc<App>, remote: &RosterBlob) -> Result<(), RelayError> {
    for (id, original) in app.store.pending_chat_identities().map_err(local_relay_error)? {
        if let Some(other) = remote.chats.iter().find(|other| other.id == id) {
            let Some(expected) = original else {
                return Err(roster_conflict(&format!("chat {} has an older creation marker without its original identity", id)));
            };
            if expected.kind != other.kind || expected.bot_ids != other.bot_ids || expected.owner_bot_id != other.owner_bot_id {
                return Err(roster_conflict(&format!("chat {} has different remote kind, members, or owner", id)));
            }
        }
    }
    Ok(())
}

async fn pull_policy_only(app: &Arc<App>, url: &str, token: &str,
    machine_file: &crate::keys::MachineFile) -> Result<(), RelayError> {
    let mut since = 0;
    loop {
        let (blobs, _) = app.relay.list_blobs(url, token, since, "policy").await?;
        if blobs.is_empty() { return Ok(()); }
        for blob in blobs {
            since = blob.seq;
            apply_blob(app, machine_file, &blob);
        }
    }
}

fn validate_merged_groups(roster: &RosterBlob) -> Result<(), RelayError> {
    for chat in &roster.chats {
        if chat.is_group() && (chat.bot_ids.is_empty() || chat.bot_ids.len() > MAX_GROUP_BOTS) {
            return Err(roster_conflict(&format!("group {} would have {} members (allowed 1–{})", chat.id, chat.bot_ids.len(), MAX_GROUP_BOTS)));
        }
    }
    Ok(())
}

fn legacy_roster_conflict(base_seq: Option<i64>, queued_seq: i64, remote_seq: i64) -> bool {
    base_seq != Some(queued_seq) && remote_seq != queued_seq
}

fn merge_ids(remote: &mut Vec<String>, base: &[String], local: &[String]) {
    remote.retain(|id| local.contains(id) || !base.contains(id));
    for id in local {
        if !base.contains(id) && !remote.contains(id) { remote.push(id.clone()); }
    }
}

// The queued snapshot describes intent relative to the last roster this Device observed.
// Compare fields, not whole entities: remote changes to other fields remain intact.
fn merge_rosters(mut remote: RosterBlob, base: &RosterBlob, queued: &RosterBlob) -> RosterBlob {
    macro_rules! merge_entities {
        ($field:ident, $($member:ident),+ $(,)?) => {
            for old in &base.$field {
                let local = queued.$field.iter().find(|entry| entry.id == old.id);
                if local.is_none() {
                    remote.$field.retain(|entry| entry.id != old.id);
                } else if let Some(current) = remote.$field.iter_mut().find(|entry| entry.id == old.id) {
                    let local = local.unwrap();
                    $(if local.$member != old.$member { current.$member = local.$member.clone(); })+
                }
            }
            for local in &queued.$field {
                if !base.$field.iter().any(|entry| entry.id == local.id)
                    && !remote.$field.iter().any(|entry| entry.id == local.id) {
                    remote.$field.push(local.clone());
                }
            }
        };
    }
    merge_entities!(bots, name, description, symbol_name, accent, avatar, runner_id, provider,
        model, thinking, legacy_instructions, workdir, capabilities);
    merge_entities!(chats, kind, title, description, owner_bot_id, is_pinned);
    for chat in &mut remote.chats {
        if let (Some(old), Some(local)) = (base.chats.iter().find(|entry| entry.id == chat.id),
            queued.chats.iter().find(|entry| entry.id == chat.id)) {
            merge_ids(&mut chat.bot_ids, &old.bot_ids, &local.bot_ids);
        }
    }
    merge_entities!(routines, bot_id, name, prompt, schedule, is_enabled, enabled_at,
        last_run_at, last_outcome, paused_reason, check);
    if queued.auto_review.is_enabled != base.auto_review.is_enabled {
        remote.auto_review.is_enabled = queued.auto_review.is_enabled;
    }
    remote.auto_review.rules.retain(|rule| queued.auto_review.rules.iter().any(|local| local.id == rule.id)
        || !base.auto_review.rules.iter().any(|old| old.id == rule.id));
    for local in &queued.auto_review.rules {
        if let Some(old) = base.auto_review.rules.iter().find(|old| old.id == local.id) {
            if let Some(current) = remote.auto_review.rules.iter_mut().find(|rule| rule.id == local.id) {
                if local != old { *current = local.clone(); }
            }
        } else if !remote.auto_review.rules.iter().any(|rule| rule.id == local.id) {
            remote.auto_review.rules.push(local.clone());
        }
    }
    remote
}

// Only creations from our submitted snapshot have become a common ancestor.
// A later remote roster can also contain unrelated creations by other Devices.
fn accepted_roster_baseline(mut base: RosterBlob, submitted: &RosterBlob) -> RosterBlob {
    macro_rules! accept_creations {
        ($field:ident) => {
            for entry in &submitted.$field {
                if !base.$field.iter().any(|old| old.id == entry.id) {
                    base.$field.push(entry.clone());
                }
            }
        };
    }
    accept_creations!(bots);
    accept_creations!(chats);
    accept_creations!(routines);
    base
}

fn rebase_queued_roster(app: &Arc<App>, machine_file: &crate::keys::MachineFile, mut item: OutboxItem) -> Result<(), RelayError> {
    let _edit = app.roster_edit.lock().unwrap();
    // Capture may predate a local edit made during network awaits.
    match app.store.queued_roster().map_err(|error| RelayError { status: None, message: error.to_string() })? {
        Some(current) if current.id != item.id => item = current,
        Some(_) => {}
        None => return Ok(()),
    }
    let local = |error: anyhow::Error| RelayError { status: None, message: error.to_string() };
    let dek = machine_file.dek().map_err(local)?;
    loop {
    let original = crate::crypto::decrypt_json::<RosterBlob>(&dek, "roster", &item.ciphertext).map_err(local)?;
    let base = app.store.roster_baseline("queued").map_err(local)?;
    let remote = app.store.roster_baseline("observed").map_err(local)?
        .map(|(_, remote)| remote).unwrap_or_default();
    if base.is_some() {
        validate_pending_chat_identities(app, &remote)?;
    }
    let mut queued = if let Some((_, base)) = base {
        merge_rosters(remote, &base, &original)
    } else {
        // Pre-baseline outbox: only upload if relay slot stayed put. Session guards that;
        // queued ciphertext is sole surviving evidence of local changes.
        original
    };
    validate_merged_groups(&queued)?;
    let pending = app.store.pending_chat_creates().map_err(local)?;
    let state = app.state.lock().unwrap();
    queued.paused = state.paused;
    queued.pause_version = state.pause_version.clone();
    queued.policy_clock = state.policy_clock;
    queued.capability_versions = state.capability_versions.clone();
    queued.policy_capabilities = state.policy_capabilities.clone();
    queued.deleted_bot_versions = state.deleted_bot_versions.clone();
    queued.bots.retain(|bot| !state.deleted_bot_versions.contains_key(&bot.id));
    for bot in &mut queued.bots {
        if let Some(capabilities) = state.policy_capabilities.get(&bot.id) { bot.capabilities = capabilities.clone(); }
    }
    queued.chats.retain(|chat| !chat.bot_ids.iter().all(|id| state.deleted_bot_versions.contains_key(id))
        && !state.group_deletes.contains(&crate::model::relay_name(&chat.id)));
    for chat in &mut queued.chats {
        chat.bot_ids.retain(|id| !state.deleted_bot_versions.contains_key(id));
        if chat.owner_bot_id.as_ref().is_some_and(|id| !chat.bot_ids.contains(id)) { chat.owner_bot_id = chat.bot_ids.first().cloned(); }
    }
    // Creation markers outlive a pull where the remote roster omitted this new chat.
    for chat in &state.chats {
        if pending.contains(&chat.meta.id) && !queued.chats.iter().any(|local| local.id == chat.meta.id)
            && !state.group_deletes.contains(&crate::model::relay_name(&chat.meta.id)) {
            queued.chats.push(chat.meta.clone());
        }
    }
    drop(state);
    validate_merged_groups(&queued)?;
    let ciphertext = crate::crypto::encrypt_json(&dek, "roster", &queued).map_err(local)?;
    let rebased = OutboxItem { id: uuid::Uuid::new_v4().to_string(), kind: "roster".into(), recipient: None,
        ciphertext, slot: Some(Slot::latest("roster")), group: None };
    let snapshot = {
        let mut state = app.state.lock().unwrap();
        remember_applied(&mut state, &rebased.id);
        state.clone()
    };
    if app.store.rebase_queued_roster_with_state(&item.id, &rebased, &snapshot).map_err(local)? {
        app.outbox_notify.notify_waiters();
        apply_roster(app, queued);
        return Ok(());
    }
    app.state.lock().unwrap().applied_blob_ids.retain(|id| id != &rebased.id);
    item = match app.store.queued_roster().map_err(local)? {
        Some(current) => current,
        None => return Ok(()),
    };
    }
}

fn apply_roster(app: &Arc<App>, mut roster: RosterBlob) {
    // Only creations still awaiting their first roster upload survive a remote omission.
    // A deleted existing chat must still lose its local transcript and outbox.
    let pending = match app.store.pending_chat_creates() {
        Ok(pending) => pending,
        Err(error) => { tracing::error!(%error, "reading pending chat creations"); return; }
    };
    let queued_ids = match app.store.queued_roster() {
        Ok(item) => item.and_then(|item| app.dek().and_then(|dek| crate::crypto::decrypt_json::<RosterBlob>(&dek, "roster", &item.ciphertext).ok()))
            .map(|queued| queued.chats.into_iter().map(|chat| chat.id).collect::<Vec<_>>()).unwrap_or_default(),
        Err(error) => { tracing::error!(%error, "reading queued roster"); return; }
    };
    let normalized_descriptions = roster.bots.iter_mut().fold(false, |changed, bot| bot.normalize_description() || changed);
    let this_device = app.this_device_id();
    let (removed, removed_bots, corrected, newly_paused, kept_checks) = {
        let mut state = app.state.lock().unwrap();
        state.policy_clock = state.policy_clock.max(roster.policy_clock);
        let mut corrected = false;
        let was_paused = state.paused;
        if roster.pause_version > state.pause_version || (roster.pause_version == PolicyVersion::default() && state.pause_version == PolicyVersion::default()) {
            state.pause_version = roster.pause_version.clone();
            state.paused = roster.paused;
        } else if roster.pause_version < state.pause_version || roster.paused != state.paused {
            corrected = true;
        }
        let newly_paused = !was_paused && state.paused;
        for (id, version) in roster.deleted_bot_versions {
            let known = state.deleted_bot_versions.entry(id).or_default();
            if version > *known { *known = version; }
        }
        for (id, version) in roster.capability_versions {
            if version > *state.capability_versions.get(&id).unwrap_or(&PolicyVersion::default()) {
                if let Some(value) = roster.policy_capabilities.get(&id).filter(|value| value.validate().is_ok()) {
                    state.policy_capabilities.insert(id.clone(), value.clone());
                    state.capability_versions.insert(id, version);
                }
            }
        }
        for bot in &mut roster.bots {
            if let Some(value) = state.policy_capabilities.get(&bot.id) {
                if bot.capabilities != *value { corrected = true; bot.capabilities = value.clone(); }
            }
        }
        let before: Vec<String> = state.bots.iter().map(|bot| bot.id.clone()).collect();
        roster.bots.retain(|bot| {
            let deleted = state.deleted_bot_versions.contains_key(&bot.id);
            if deleted { corrected = true; }
            !deleted
        });
        roster.chats.retain(|chat| !chat.bot_ids.iter().all(|id| state.deleted_bot_versions.contains_key(id))
            && !state.group_deletes.contains(&crate::model::relay_name(&chat.id)));
        for chat in &mut roster.chats {
            chat.bot_ids.retain(|id| !state.deleted_bot_versions.contains_key(id));
            if chat.owner_bot_id.as_ref().is_some_and(|id| !chat.bot_ids.contains(id)) { chat.owner_bot_id = chat.bot_ids.first().cloned(); }
        }
        roster.routines.retain(|routine| !state.deleted_bot_versions.contains_key(&routine.bot_id));
        let removed_bots: Vec<String> = before.into_iter().filter(|id| !roster.bots.iter().any(|bot| bot.id == *id)).collect();
        let kept_checks = this_device.is_some_and(|this| crate::routines::keep_checks(&state.routines, &mut roster.routines, &roster.bots, &this));
        state.bots = roster.bots;
        state.routines = roster.routines;
        state.auto_review = roster.auto_review;
        let incoming_ids: Vec<String> = roster.chats.iter().map(|c| c.id.clone()).collect();
        let removed: Vec<String> = state.chats.iter().filter(|c| !incoming_ids.contains(&c.meta.id)
            && !(pending.contains(&c.meta.id) && queued_ids.contains(&c.meta.id)
                && !state.group_deletes.contains(&crate::model::relay_name(&c.meta.id))))
            .map(|c| c.meta.id.clone()).collect();
        state.chats.retain(|c| !removed.contains(&c.meta.id));
        for meta in roster.chats {
            match state.chats.iter_mut().find(|c| c.meta.id == meta.id) {
                Some(chat) => chat.meta = meta,
                None => state.chats.push(Chat { meta, unread_count: 0, usage: None, compactions: Vec::new() }),
            }
        }
        (removed, removed_bots, corrected, newly_paused, kept_checks)
    };
    if newly_paused { app.stop_for_pause(); }
    let snapshot = app.state.lock().unwrap().clone();
    if let Err(error) = app.store.save_state_deleting_chats(&snapshot, &removed) {
        tracing::error!(%error, "saving synced roster");
    }
    let bot_ids: Vec<String> = snapshot.bots.iter().map(|bot| bot.id.clone()).collect();
    if let Err(error) = app.store.retain_codemode_bots(&bot_ids) {
        tracing::warn!(%error, "forgetting deleted bots' script values");
    }
    crate::runtime::cancel_removed_bots(app, &removed_bots);
    for chat_id in removed {
        app.cancel_chat(&chat_id);
        app.emit(Event::ChatRemoved { chat_id });
    }
    #[cfg(feature = "runner")]
    app.shell_sessions.close_orphans(app);
    app.roster_changed(normalized_descriptions || kept_checks || corrected);
}

fn apply_chat_op(app: &Arc<App>, op: ChatBlob) {
    match op {
        ChatBlob::Upsert { message } => {
            #[cfg(feature = "runner")]
            let heard = (message.author == Author::You && app.message(&message.chat_id, &message.id).is_none())
                .then(|| message.clone());
            {
                let mut state = app.state.lock().unwrap();
                if !state.chats.iter().any(|c| c.meta.id == message.chat_id) {
                    // Roster not here yet: keep the message under a placeholder until it is.
                    state.chats.push(Chat {
                        meta: ChatMeta { id: message.chat_id.clone(), kind: "group".into(), title: Some("Chat".into()), bot_ids: vec![], owner_bot_id: None, description: None, is_pinned: false, created_at: message.created_at },
                        unread_count: 0,
                        usage: None,
                        compactions: Vec::new(),
                    });
                }
            }
            // The cycle saves state once after the page.
            app.upsert_message(message, false);
            #[cfg(feature = "runner")]
            if let Some(message) = heard {
                crate::turns::hear_user_message(app, &message);
            }
        }
        ChatBlob::Remove { chat_id, message_id } => app.remove_message(&chat_id, &message_id, false),
        ChatBlob::ClearUnread { chat_id } => app.mark_read(&chat_id, false),
    }
}

/// Restore path: pull the account DEK the identity device sealed to the content key.
pub async fn fetch_dek(app: &Arc<App>, url: &str, identity: &crate::keys::Identity, machine: &crate::keys::Machine) -> Result<[u8; 32], String> {
    let token = app.relay.authenticate(url, machine).await.map_err(|e| e.to_string())?;
    let (blobs, _) = app.relay.list_blobs(url, &token, 0, "key").await.map_err(|e| e.to_string())?;
    for blob in blobs.iter().rev() {
        let Ok(ciphertext) = unb64(&blob.ciphertext) else { continue };
        if let Ok(bytes) = crate::crypto::unseal(&identity.content_secret, &ciphertext) {
            if let Ok(dek) = <[u8; 32]>::try_from(bytes.as_slice()) {
                return Ok(dek);
            }
        }
    }
    Err("The relay has no account key for this identity. Create the identity on a Device that is online first.".into())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::config::Config;

    struct ScratchApp(Arc<App>, std::path::PathBuf);

    impl Drop for ScratchApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn scratch_app() -> ScratchApp {
        let home = std::env::temp_dir().join(format!("lorca-sync-{}", uuid::Uuid::new_v4()));
        let app = App::load(Config { home: home.clone(), port: 0 }).unwrap();
        ScratchApp(app, home)
    }

    #[tokio::test]
    async fn a_device_without_an_account_does_not_wait() {
        let scratch = scratch_app();
        let waited = tokio::time::timeout(Duration::from_secs(5), wait_for_account(&scratch.0, ACCOUNT_WAIT)).await;
        assert_eq!(waited, Ok(false));
    }

    #[tokio::test]
    async fn the_wait_ends_when_the_account_this_device_holds_is_pulled() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let machine_file = app.machine_file().unwrap();

        // The pull of an account this Device held before does not count.
        app.account_pulled.send_replace(Some(crate::keys::Machine::generate().pubkey()));
        assert!(!wait_for_account(app, Duration::from_millis(50)).await);

        let waiting = tokio::spawn({
            let app = app.clone();
            async move { wait_for_account(&app, ACCOUNT_WAIT).await }
        });
        tokio::task::yield_now().await;
        mark_account_pulled(app, &machine_file);
        assert!(tokio::time::timeout(Duration::from_secs(5), waiting).await.unwrap().unwrap());

        // Pulled once, it answers at once.
        assert!(wait_for_account(app, Duration::from_millis(1)).await);
    }

    /// A machine the relay lists that never sent its `machine` blob shows as unknown once the
    /// pull has caught up and ten minutes have passed since the relay attested it. Its blob
    /// makes it a Device like any other, and a key the relay stops listing leaves at once.
    #[tokio::test]
    async fn a_listed_machine_that_never_said_what_it_is_shows_as_unknown() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let unknown = || -> Vec<String> {
            let snapshot = app.snapshot();
            snapshot["devices"].as_array().unwrap().iter().filter(|d| d["unknown"] == true).map(|d| d["id"].as_str().unwrap().to_string()).collect()
        };
        let (silent, fresh) = (crate::keys::Machine::generate().pubkey(), crate::keys::Machine::generate().pubkey());
        let now = crate::config::now_unix();
        {
            let mut state = app.state.lock().unwrap();
            state.listed_machines.insert(silent.clone(), now - UNKNOWN_AFTER - 5);
            state.listed_machines.insert(fresh.clone(), now - 5);
            state.listed_machines.insert(app.this_device_id().unwrap(), now - UNKNOWN_AFTER - 5);
        }
        assert!(!settle_unknown_machines(app), "a Device still reading the log takes nobody for unknown");
        assert!(unknown().is_empty());

        app.state.lock().unwrap().caught_up = true;
        assert!(settle_unknown_machines(app));
        assert_eq!(unknown(), [silent.clone()]);
        let snapshot = app.snapshot();
        let entry = snapshot["devices"].as_array().unwrap().iter().find(|d| d["id"] == silent.as_str()).unwrap();
        assert_eq!((entry["name"].as_str(), entry["os"].as_str(), entry["is_this_device"].as_bool()), (Some(""), Some(""), Some(false)));

        let device = Device { id: silent.clone(), name: "Laptop".into(), model: String::new(), os: "windows".into(), os_version: String::new(), box_pubkey: String::new(), plugins: Vec::new(), updated_at: now };
        upsert_device(&mut app.state.lock().unwrap().devices, device);
        assert!(unknown().is_empty(), "its blob landed");
        assert!(settle_unknown_machines(app));

        app.state.lock().unwrap().listed_machines.insert(fresh.clone(), now - UNKNOWN_AFTER - 1);
        assert!(settle_unknown_machines(app));
        assert_eq!(unknown(), [fresh.clone()]);
        app.state.lock().unwrap().listed_machines.remove(&fresh);
        assert!(unknown().is_empty(), "unpaired from another Device");
    }
    fn bot(id: &str) -> Bot {
        Bot {
            id: id.into(), name: id.into(), description: String::new(), symbol_name: String::new(), accent: String::new(), avatar: None,
            runner_id: "runner".into(), provider: "deepseek".into(), model: None, thinking: None,
            legacy_instructions: String::new(), workdir: None, capabilities: Capabilities::default(), created_at: 1.0,
        }
    }

    fn roster(bot: Bot) -> RosterBlob {
        RosterBlob { bots: vec![bot], ..Default::default() }
    }

    fn policy(device: &str, counter: u64, paused: Option<bool>, capabilities: Option<Capabilities>) -> PolicyBlob {
        PolicyBlob { paused, bot_id: capabilities.as_ref().map(|_| "bot".into()), capabilities, removed: false,
            version: PolicyVersion { counter, device_id: device.into() } }
    }

    #[test]
    fn two_devices_reconcile_pause_and_explicit_newer_resume_after_stale_roster_upload() {
        let a = scratch_app();
        let b = scratch_app();
        let stale = roster(bot("bot"));
        apply_roster(&a.0, stale.clone());
        apply_roster(&b.0, stale.clone());
        let pause = policy("A", 1, Some(true), None);
        apply_policy(&a.0, pause.clone());
        apply_policy(&b.0, pause);
        apply_roster(&b.0, stale.clone()); // B's offline edit lands after A's Pause.
        assert!(a.0.is_paused() && b.0.is_paused());
        let resume = policy("B", 2, Some(false), None);
        apply_policy(&b.0, resume.clone());
        apply_policy(&a.0, resume);
        apply_roster(&a.0, RosterBlob { paused: true, pause_version: PolicyVersion { counter: 1, device_id: "A".into() }, ..stale });
        assert!(!a.0.is_paused() && !b.0.is_paused(), "only newer explicit resume lifts Pause");
    }

    #[test]
    fn two_devices_preserve_restricted_capabilities_through_stale_roster() {
        let a = scratch_app();
        let b = scratch_app();
        let stale = roster(bot("bot"));
        apply_roster(&a.0, stale.clone());
        apply_roster(&b.0, stale.clone());
        let restricted = Capabilities { shell: false, write: false, plugins: Some(vec![]) };
        let restriction = policy("A", 1, None, Some(restricted.clone()));
        apply_policy(&a.0, restriction.clone());
        apply_policy(&b.0, restriction);
        apply_roster(&b.0, stale.clone());
        assert_eq!(b.0.bot("bot").unwrap().capabilities, restricted);
        let relaxed = policy("B", 2, None, Some(Capabilities::default()));
        apply_policy(&a.0, relaxed.clone());
        apply_policy(&b.0, relaxed);
        apply_roster(&b.0, stale);
        assert!(b.0.bot("bot").unwrap().capabilities.shell, "newer explicit relax wins");
    }

    #[test]
    fn removed_bot_does_not_return_from_stale_surviving_group_roster() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let mut stale = roster(bot("bot"));
        stale.bots.push(bot("other"));
        stale.chats.push(ChatMeta { id: "group".into(), kind: "group".into(), title: None,
            bot_ids: vec!["bot".into(), "other".into()], owner_bot_id: Some("bot".into()), description: None, is_pinned: false, created_at: 1.0 });
        apply_roster(app, stale.clone());
        apply_policy(app, PolicyBlob { paused: None, bot_id: Some("bot".into()), capabilities: None, removed: true,
            version: PolicyVersion { counter: 1, device_id: "A".into() } });
        apply_roster(app, stale);
        assert!(app.bot("bot").is_none());
        let group = app.chat("group").unwrap();
        assert_eq!(group.meta.bot_ids, vec!["other".to_string()]);
        assert_eq!(group.meta.owner_bot_id.as_deref(), Some("other"));
    }

    #[test]
    fn queued_offline_bot_edit_keeps_profile_but_not_stale_policy() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let initial_upload = app.store.queued_roster().unwrap().unwrap();
        app.store.remove_outbox_roster_with_state(&initial_upload.id, &app.state.lock().unwrap().clone(), &[]).unwrap();
        app.store.observe_roster(1, &roster(bot("bot"))).unwrap();
        let mut queued = roster(bot("bot"));
        queued.bots[0].name = "Offline edit".into();
        queued.paused = false;
        let dek = app.dek().unwrap();
        let ciphertext = crate::crypto::encrypt_json(&dek, "roster", &queued).unwrap();
        app.push_slot_blob("roster", Slot::latest("roster"), None, ciphertext);
        let pending = app.store.queued_roster().unwrap().unwrap();
        apply_policy(app, policy("A", 1, Some(true), None));
        apply_policy(app, policy("A", 2, None, Some(Capabilities { shell: false, write: false, plugins: None })));
        rebase_queued_roster(app, &app.machine_file().unwrap(), pending).unwrap();
        let published = app.store.queued_roster().unwrap().unwrap();
        let projected: RosterBlob = crate::crypto::decrypt_json(&dek, "roster", &published.ciphertext).unwrap();
        assert_eq!(projected.bots[0].name, "Offline edit");
        assert!(projected.paused);
        assert!(!projected.bots[0].capabilities.shell);
        assert_eq!(app.bot("bot").unwrap().name, "Offline edit");
        assert!(app.is_paused());
    }
    #[test]
    fn pre_baseline_queued_roster_keeps_edit_and_refuses_remote_conflict() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let queued = app.store.queued_roster().unwrap().unwrap();
        app.store.forget_queued_roster_base().unwrap();
        assert!(app.store.roster_baseline("queued").unwrap().is_none());
        assert!(!legacy_roster_conflict(None, 0, 0));
        assert!(legacy_roster_conflict(None, 0, 7));
        let original: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &queued.ciphertext).unwrap();
        rebase_queued_roster(app, &app.machine_file().unwrap(), queued).unwrap();
        let rebased = app.store.queued_roster().unwrap().unwrap();
        let projected: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &rebased.ciphertext).unwrap();
        assert_eq!(projected.bots, original.bots);
    }

    #[test]
    fn local_group_edit_during_pull_rebases_newest_slot_and_preserves_transcript() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let initial_upload = app.store.queued_roster().unwrap().unwrap();
        app.store.remove_outbox_roster_with_state(&initial_upload.id, &app.state.lock().unwrap().clone(), &[]).unwrap();
        let mut base = roster(bot("A"));
        base.bots.extend([bot("B"), bot("C")]);
        base.chats.push(ChatMeta { id: "group".into(), kind: "group".into(), title: None,
            bot_ids: vec!["A".into(), "B".into(), "C".into()], owner_bot_id: Some("C".into()), description: None, is_pinned: false, created_at: 1.0 });
        app.store.observe_roster(1, &base).unwrap();
        apply_roster(app, base.clone());
        let message = Message::new("group", Author::You, Body::text("keep transcript"));
        app.upsert_message(message.clone(), true);
        app.update_chat_meta("group", |chat| chat.bot_ids.retain(|id| id != "A")).unwrap();
        let captured = app.store.queued_roster().unwrap().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let editor = std::thread::spawn({
            let app = app.clone();
            let barrier = barrier.clone();
            move || {
                barrier.wait();
                app.update_chat_meta("group", |chat| chat.bot_ids.retain(|id| id != "B")).unwrap();
            }
        });
        barrier.wait();
        editor.join().unwrap();
        let latest = app.store.queued_roster().unwrap().unwrap();
        assert_ne!(captured.id, latest.id);
        let mut remote = base;
        remote.bots.push(bot("D"));
        app.store.observe_roster(2, &remote).unwrap();
        rebase_queued_roster(app, &app.machine_file().unwrap(), captured).unwrap();
        let published = app.store.queued_roster().unwrap().unwrap();
        let merged: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &published.ciphertext).unwrap();
        assert_eq!(merged.chats[0].bot_ids, ["C"]);
        assert_eq!(app.chat("group").unwrap().meta.bot_ids, ["C"]);
        assert!(merged.bots.iter().any(|bot| bot.id == "D"));
        assert!(app.message("group", &message.id).is_some());
    }

    #[test]
    fn rebase_cas_rejects_stale_slot_without_advancing_baseline() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let stale = app.store.queued_roster().unwrap().unwrap();
        let replacement = crate::crypto::encrypt_json(&app.dek().unwrap(), "roster", &roster(bot("new"))).unwrap();
        app.push_slot_blob("roster", Slot::latest("roster"), None, replacement.clone());
        let mut remote = roster(bot("remote"));
        remote.updated_at = 2.0;
        app.store.observe_roster(5, &remote).unwrap();
        let queued_base = app.store.roster_baseline("queued").unwrap();
        let candidate = OutboxItem { id: "rebased".into(), kind: "roster".into(), recipient: None,
            ciphertext: stale.ciphertext, slot: Some(Slot::latest("roster")), group: None };
        assert!(!app.store.rebase_queued_roster_with_state(&stale.id, &candidate, &app.state.lock().unwrap().clone()).unwrap());
        assert_eq!(app.store.queued_roster().unwrap().unwrap().ciphertext, replacement);
        assert_eq!(app.store.roster_baseline("queued").unwrap().unwrap().0, queued_base.unwrap().0);
    }

    #[test]
    fn incoming_policy_during_rebase_preserves_newer_pause() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let initial_upload = app.store.queued_roster().unwrap().unwrap();
        app.store.remove_outbox_roster_with_state(&initial_upload.id, &app.state.lock().unwrap().clone(), &[]).unwrap();
        let base = roster(bot("bot"));
        app.store.observe_roster(1, &base).unwrap();
        apply_roster(app, base);
        app.update_bot("bot", |bot| bot.name = "Offline name".into()).unwrap();
        let captured = app.store.queued_roster().unwrap().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let policy_thread = std::thread::spawn({
            let app = app.clone();
            let barrier = barrier.clone();
            move || {
                barrier.wait();
                apply_policy(&app, policy("remote", 5, Some(true), None));
            }
        });
        barrier.wait();
        policy_thread.join().unwrap();
        rebase_queued_roster(app, &app.machine_file().unwrap(), captured).unwrap();
        let item = app.store.queued_roster().unwrap().unwrap();
        let merged: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &item.ciphertext).unwrap();
        assert!(merged.paused);
        assert_eq!(merged.pause_version.counter, 5);
        assert_eq!(merged.bots[0].name, "Offline name");
        assert!(app.is_paused());
    }

    #[test]
    fn offline_chat_and_queued_message_file_survive_remote_roster_without_chat() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        apply_roster(app, roster(bot("bot")));
        let chat = app.create_chat(ChatMeta { id: "offline".into(), kind: "dm".into(), title: None,
            bot_ids: vec!["bot".into()], owner_bot_id: Some("bot".into()), description: None, is_pinned: false, created_at: 1.0 }).unwrap();
        let message = Message::new(&chat.meta.id, Author::You, Body::text("offline transcript"));
        app.upsert_message(message.clone(), true);
        let file = OutboxItem { id: "offline-file".into(), kind: "file".into(), recipient: None,
            ciphertext: b"encrypted-file".to_vec(), slot: None, group: Some(crate::model::relay_name("offline")) };
        app.store.queue_outbox(&file).unwrap();
        let pending = app.store.queued_roster().unwrap().unwrap();
        apply_policy(app, policy("A", 1, Some(true), None));
        apply_roster(app, roster(bot("bot"))); // Remote pull before any upload.
        assert!(app.message("offline", &message.id).is_some());
        assert!(app.store.outbox().unwrap().iter().any(|item| item.id == file.id));
        rebase_queued_roster(app, &app.machine_file().unwrap(), pending).unwrap();
        let published = app.store.queued_roster().unwrap().unwrap();
        let roster: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &published.ciphertext).unwrap();
        assert!(roster.chats.iter().any(|chat| chat.id == "offline"));
        assert!(roster.paused && app.is_paused());
        assert!(app.message("offline", &message.id).is_some());
        let waiting = app.store.outbox().unwrap();
        assert!(waiting.iter().any(|item| item.kind == "chat" && item.group == file.group));
        assert!(waiting.iter().any(|item| item.id == file.id && item.ciphertext == file.ciphertext));
        assert!(app.store.pending_chat_creates().unwrap().contains(&"offline".to_string()));
        app.store.remove_outbox_roster_with_state(&published.id, &app.state.lock().unwrap().clone(), &["offline".into()]).unwrap();
        // Server acceptance acknowledges creation despite a newer queued slot.
        if let Some(latest) = app.store.queued_roster().unwrap() {
            assert!(!app.store.pending_chat_creates().unwrap().contains(&"offline".to_string()));
            app.store.remove_outbox_roster_with_state(&latest.id, &app.state.lock().unwrap().clone(), &[]).unwrap();
        }
        assert!(!app.store.pending_chat_creates().unwrap().contains(&"offline".to_string()));
    }

    #[test]
    fn offline_existing_edits_survive_remote_additions_and_restart() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let initial_upload = app.store.queued_roster().unwrap().unwrap();
        app.store.remove_outbox_roster_with_state(&initial_upload.id, &app.state.lock().unwrap().clone(), &[]).unwrap();
        let mut initial = roster(bot("lead"));
        initial.chats.push(ChatMeta { id: "group".into(), kind: "group".into(), title: Some("Old".into()),
            bot_ids: vec!["lead".into()], owner_bot_id: Some("lead".into()), description: None, is_pinned: false, created_at: 1.0 });
        initial.routines.push(Routine { id: "routine".into(), bot_id: "lead".into(), name: "Daily".into(),
            prompt: "Check".into(), schedule: "every 1d".into(), is_enabled: true, enabled_at: 1.0,
            last_run_at: None, last_outcome: None, paused_reason: None, check: None, created_at: 1.0 });
        app.store.observe_roster(4, &initial).unwrap();
        apply_roster(app, initial.clone());
        let message = Message::new("group", Author::You, Body::text("keep transcript"));
        app.upsert_message(message.clone(), true);
        {
            let mut state = app.state.lock().unwrap();
            let group = state.chats.iter_mut().find(|chat| chat.meta.id == "group").unwrap();
            group.meta.title = Some("Renamed".into());
            group.meta.is_pinned = true;
            state.routines.clear();
        }
        app.push_roster();
        let reopened = App::load(Config { home: scratch.1.clone(), port: 0 }).unwrap();
        let mut remote = initial;
        remote.bots.push(bot("new-bot"));
        remote.chats.push(ChatMeta { id: "new-chat".into(), kind: "dm".into(), title: None,
            bot_ids: vec!["new-bot".into()], owner_bot_id: Some("new-bot".into()), description: None, is_pinned: false, created_at: 2.0 });
        reopened.store.observe_roster(8, &remote).unwrap();
        let pending = reopened.store.queued_roster().unwrap().unwrap();
        let queued: RosterBlob = crate::crypto::decrypt_json(&reopened.dek().unwrap(), "roster", &pending.ciphertext).unwrap();
        apply_roster(&reopened, merge_rosters(remote, &reopened.store.roster_baseline("queued").unwrap().unwrap().1, &queued));
        rebase_queued_roster(&reopened, &reopened.machine_file().unwrap(), pending).unwrap();
        let uploaded = reopened.store.queued_roster().unwrap().unwrap();
        let result: RosterBlob = crate::crypto::decrypt_json(&reopened.dek().unwrap(), "roster", &uploaded.ciphertext).unwrap();
        assert_eq!(result.chats.iter().find(|chat| chat.id == "group").unwrap().title.as_deref(), Some("Renamed"));
        assert!(result.chats.iter().find(|chat| chat.id == "group").unwrap().is_pinned);
        assert!(!result.routines.iter().any(|routine| routine.id == "routine"));
        assert!(result.bots.iter().any(|bot| bot.id == "new-bot"));
        assert!(result.chats.iter().any(|chat| chat.id == "new-chat"));
        assert!(reopened.message("group", &message.id).is_some());
    }

    #[test]
    fn remote_deletion_of_existing_chat_removes_transcript_and_queued_blobs() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let initial_upload = app.store.queued_roster().unwrap().unwrap();
        app.store.remove_outbox_roster_with_state(&initial_upload.id, &app.state.lock().unwrap().clone(), &[]).unwrap();
        let mut remote = roster(bot("bot"));
        remote.chats.push(ChatMeta { id: "existing".into(), kind: "dm".into(), title: None,
            bot_ids: vec!["bot".into()], owner_bot_id: Some("bot".into()), description: None, is_pinned: false, created_at: 1.0 });
        app.store.observe_roster(1, &remote).unwrap();
        apply_roster(app, remote);
        let message = Message::new("existing", Author::You, Body::text("must delete"));
        app.upsert_message(message.clone(), true);
        app.store.queue_outbox(&OutboxItem { id: "existing-file".into(), kind: "file".into(), recipient: None,
            ciphertext: b"encrypted-file".to_vec(), slot: None, group: Some(crate::model::relay_name("existing")) }).unwrap();
        app.push_roster(); // A queued edit is not evidence of a locally created chat.
        let pending = app.store.queued_roster().unwrap().unwrap();
        app.store.observe_roster(2, &roster(bot("bot"))).unwrap();
        apply_roster(app, roster(bot("bot")));
        rebase_queued_roster(app, &app.machine_file().unwrap(), pending).unwrap();
        assert!(app.chat("existing").is_none());
        assert!(app.message("existing", &message.id).is_none());
        assert!(!app.store.outbox().unwrap().iter().any(|item| item.group.as_deref() == Some("existing")));
        let published = app.store.queued_roster().unwrap().unwrap();
        let roster: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &published.ciphertext).unwrap();
        assert!(!roster.chats.iter().any(|chat| chat.id == "existing"));
    }

    #[test]
    fn remote_deleted_existing_chat_stays_deleted_despite_offline_rename() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let initial_upload = app.store.queued_roster().unwrap().unwrap();
        app.store.remove_outbox_roster_with_state(&initial_upload.id, &app.state.lock().unwrap().clone(), &[]).unwrap();
        let mut initial = roster(bot("bot"));
        initial.chats.push(ChatMeta { id: "old".into(), kind: "group".into(), title: Some("Before".into()),
            bot_ids: vec!["bot".into()], owner_bot_id: Some("bot".into()), description: None, is_pinned: false, created_at: 1.0 });
        app.store.observe_roster(1, &initial).unwrap();
        apply_roster(app, initial);
        let message = Message::new("old", Author::You, Body::text("deleted with group"));
        app.upsert_message(message.clone(), true);
        app.state.lock().unwrap().chats[0].meta.title = Some("Offline rename".into());
        app.push_roster();
        let remote = roster(bot("bot"));
        app.store.observe_roster(2, &remote).unwrap();
        apply_roster(app, remote);
        let pending = app.store.queued_roster().unwrap().unwrap();
        rebase_queued_roster(app, &app.machine_file().unwrap(), pending).unwrap();
        let result = app.store.queued_roster().unwrap().unwrap();
        let projected: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &result.ciphertext).unwrap();
        assert!(!projected.chats.iter().any(|chat| chat.id == "old"));
        assert!(app.message("old", &message.id).is_none());
    }

    #[test]
    fn queued_group_deletion_does_not_resurrect_from_remote_roster() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        apply_roster(app, roster(bot("bot")));
        let chat = ChatMeta { id: "deleted".into(), kind: "dm".into(), title: None,
            bot_ids: vec!["bot".into()], owner_bot_id: Some("bot".into()), description: None, is_pinned: false, created_at: 1.0 };
        app.create_chat(chat.clone()).unwrap();
        let queued = app.store.queued_roster().unwrap().unwrap();
        app.delete_chat("deleted");
        let mut remote = roster(bot("bot"));
        remote.chats.push(chat);
        apply_roster(app, remote);
        rebase_queued_roster(app, &app.machine_file().unwrap(), queued).unwrap();
        assert!(app.chat("deleted").is_none());
        assert!(app.state.lock().unwrap().group_deletes.contains(&crate::model::relay_name("deleted")));
        let published = app.store.queued_roster().unwrap().unwrap();
        let roster: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &published.ciphertext).unwrap();
        assert!(!roster.chats.iter().any(|chat| chat.id == "deleted"));
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn stale_roster_conflicts_and_preserves_other_devices_chat() {
        use axum::{extract::{Query, State as HttpState}, http::StatusCode, routing::put, Json, Router};
        use serde_json::{json, Value};
        use tokio::sync::Mutex;

        #[derive(Default)]
        struct RelaySlot { seq: i64, roster: Option<Value> }
        async fn upload(HttpState(slot): HttpState<Arc<Mutex<RelaySlot>>>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
            let mut slot = slot.lock().await;
            if body["kind"] == "roster" {
                if slot.roster.as_ref().is_some_and(|current| current["id"] == body["id"]) {
                    return (StatusCode::OK, Json(json!({ "seq": slot.seq })));
                }
                if body["expected_slot_seq"].as_i64() != Some(slot.roster.as_ref().map_or(0, |current| current["seq"].as_i64().unwrap())) {
                    return (StatusCode::CONFLICT, Json(json!({ "error": "Roster slot changed" })));
                }
            }
            slot.seq += 1;
            let seq = slot.seq;
            if body["kind"] == "roster" {
                slot.roster = Some(json!({ "id": body["id"], "kind": "roster", "recipient_machine_pubkey": null,
                    "ciphertext": body["ciphertext"], "seq": seq, "created_at": 1 }));
            }
            (StatusCode::OK, Json(json!({ "seq": seq })))
        }
        async fn list(HttpState(slot): HttpState<Arc<Mutex<RelaySlot>>>, Query(query): Query<std::collections::HashMap<String, String>>) -> Json<Value> {
            let slot = slot.lock().await;
            let since: i64 = query["since"].parse().unwrap();
            let blobs: Vec<Value> = slot.roster.iter().filter(|blob| blob["seq"].as_i64().unwrap() > since).cloned().collect();
            Json(json!({ "blobs": blobs, "seq": slot.seq }))
        }
        let slot = Arc::new(Mutex::new(RelaySlot { seq: 1, roster: None }));
        let server = Router::new().route("/v1/blobs", put(upload).get(list)).with_state(slot);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
        let a = scratch_app();
        let b = scratch_app();
        crate::identity::create(&a.0, Some("A".into())).unwrap();
        *b.0.machine.lock().unwrap() = a.0.machine_file(); // Account DEK shared between Devices.
        let runner = b.0.local_device().unwrap();
        b.0.state.lock().unwrap().devices.push(runner);
        let mut baseline = roster(bot("bot"));
        baseline.chats.push(ChatMeta { id: "group".into(), kind: "group".into(), title: Some("Original".into()),
            bot_ids: vec!["bot".into()], owner_bot_id: Some("bot".into()), description: None, is_pinned: false, created_at: 1.0 });
        baseline.routines.push(Routine { id: "routine".into(), bot_id: "bot".into(), name: "Daily".into(),
            prompt: "Check".into(), schedule: "every 1d".into(), is_enabled: true, enabled_at: 1.0,
            last_run_at: None, last_outcome: None, paused_reason: None, check: None, created_at: 1.0 });
        let initial_queue = a.0.store.queued_roster().unwrap().unwrap();
        a.0.store.remove_outbox_roster_with_state(&initial_queue.id, &a.0.state.lock().unwrap().clone(), &[]).unwrap();
        for app in [&a.0, &b.0] {
            apply_roster(app, baseline.clone());
            app.state.lock().unwrap().last_seq = 1; // Focus on roster pull, not initial transcript paging.
        }
        a.0.store.observe_roster(0, &baseline).unwrap();
        b.0.store.observe_roster(0, &baseline).unwrap();
        let group_message = Message::new("group", Author::You, Body::text("G transcript"));
        a.0.upsert_message(group_message.clone(), true);
        {
            let mut state = a.0.state.lock().unwrap();
            let group = state.chats.iter_mut().find(|chat| chat.meta.id == "group").unwrap();
            group.meta.title = Some("Offline rename".into());
            group.meta.is_pinned = true;
            state.routines.clear();
        }
        a.0.push_roster();
        for (app, id) in [(&a.0, "offline"), (&b.0, "x")] {
            app.create_chat(ChatMeta { id: id.into(), kind: "dm".into(), title: None,
                bot_ids: vec!["bot".into()], owner_bot_id: Some("bot".into()), description: None, is_pinned: false, created_at: 1.0 }).unwrap();
        }
        let mut new_bot = bot("new-bot");
        new_bot.runner_id = b.0.this_device_id().unwrap();
        b.0.create_bot_with_dm(new_bot, None).unwrap();
        let b_roster = b.0.store.queued_roster().unwrap().unwrap();
        let b_seq = b.0.relay.put_blob(&url, "token", b_roster.clone(), 0).await.unwrap();
        b.0.state.lock().unwrap().roster_slot_seq = b_seq;
        b.0.store.remove_outbox_roster_with_state(&b_roster.id, &b.0.state.lock().unwrap().clone(), &["x".into()]).unwrap();
        assert!(!b.0.store.pending_chat_creates().unwrap().contains(&"x".to_string()));
        let message = Message::new("x", Author::You, Body::text("X remains"));
        b.0.upsert_message(message.clone(), true);
        let file = OutboxItem { id: "x-file".into(), kind: "file".into(), recipient: None,
            ciphertext: b"encrypted".to_vec(), slot: None, group: Some(crate::model::relay_name("x")) };
        b.0.store.queue_outbox(&file).unwrap();
        let stale = a.0.store.queued_roster().unwrap().unwrap();
        assert_eq!(a.0.relay.put_blob(&url, "token", stale.clone(), 0).await.unwrap_err().status, Some(409));
        let offline = Message::new("offline", Author::You, Body::text("offline"));
        a.0.upsert_message(offline.clone(), true);
        let offline_file = OutboxItem { id: "offline-file".into(), kind: "file".into(), recipient: None,
            ciphertext: b"encrypted".to_vec(), slot: None, group: Some(crate::model::relay_name("offline")) };
        a.0.store.queue_outbox(&offline_file).unwrap();
        assert!(a.0.store.pending_chat_creates().unwrap().contains(&"offline".to_string()));
        pull_blobs(&a.0, &url, "token", &a.0.machine_file().unwrap()).await.unwrap();
        rebase_queued_roster(&a.0, &a.0.machine_file().unwrap(), stale).unwrap();
        let merged = a.0.store.queued_roster().unwrap().unwrap();
        let roster: RosterBlob = crate::crypto::decrypt_json(&a.0.dek().unwrap(), "roster", &merged.ciphertext).unwrap();
        assert!(roster.chats.iter().any(|chat| chat.id == "x"));
        assert!(roster.chats.iter().any(|chat| chat.id == "offline"));
        assert_eq!(roster.chats.iter().find(|chat| chat.id == "group").unwrap().title.as_deref(), Some("Offline rename"));
        assert!(roster.chats.iter().find(|chat| chat.id == "group").unwrap().is_pinned);
        assert!(!roster.routines.iter().any(|routine| routine.id == "routine"));
        assert!(roster.bots.iter().any(|bot| bot.name == "new-bot"));
        let expected = a.0.state.lock().unwrap().roster_slot_seq;
        let seq = a.0.relay.put_blob(&url, "token", merged.clone(), expected).await.unwrap();
        a.0.state.lock().unwrap().roster_slot_seq = seq;
        a.0.store.remove_outbox_roster_with_state(&merged.id, &a.0.state.lock().unwrap().clone(), &["offline".into(), "x".into()]).unwrap();
        assert!(!a.0.store.pending_chat_creates().unwrap().contains(&"offline".to_string()));
        pull_blobs(&b.0, &url, "token", &b.0.machine_file().unwrap()).await.unwrap();
        assert!(b.0.chat("x").is_some());
        assert!(b.0.message("x", &message.id).is_some());
        assert!(b.0.store.outbox().unwrap().iter().any(|item| item.id == file.id));
        assert!(a.0.message("offline", &offline.id).is_some());
        assert_eq!(b.0.chat("group").unwrap().meta.title.as_deref(), Some("Offline rename"));
        assert!(!b.0.state.lock().unwrap().routines.iter().any(|routine| routine.id == "routine"));
        assert!(a.0.message("group", &group_message.id).is_some());
        assert!(a.0.store.outbox().unwrap().iter().any(|item| item.id == offline_file.id));
        task.abort();
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn cas_membership_conflicts_preserve_local_and_remote() {
        use axum::{extract::{Query, State as HttpState}, http::StatusCode, routing::put, Json, Router};
        use serde_json::{json, Value};
        use tokio::sync::Mutex;
        async fn upload(HttpState(slot): HttpState<Arc<Mutex<(i64, Option<Value>)>>>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
            let mut slot = slot.lock().await;
            if body["expected_slot_seq"].as_i64() != Some(slot.0) {
                return (StatusCode::CONFLICT, Json(json!({"error":"Roster slot changed"})));
            }
            slot.0 += 1;
            let seq = slot.0;
            slot.1 = Some(json!({"id":body["id"],"kind":"roster","recipient_machine_pubkey":null,
                "ciphertext":body["ciphertext"],"seq":seq,"created_at":1}));
            (StatusCode::OK, Json(json!({"seq":seq})))
        }
        async fn list(HttpState(slot): HttpState<Arc<Mutex<(i64, Option<Value>)>>>, Query(query): Query<std::collections::HashMap<String,String>>) -> Json<Value> {
            let slot = slot.lock().await;
            Json(json!({"blobs":slot.1.iter().filter(|blob| blob["seq"].as_i64().unwrap() > query["since"].parse::<i64>().unwrap()).cloned().collect::<Vec<_>>(),"seq":slot.0}))
        }
        for case in ["removals", "additions", "legacy"] {
            let slot = Arc::new(Mutex::new((0, None)));
            let server = Router::new().route("/v1/blobs", put(upload).get(list)).with_state(slot.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
            let scratch = scratch_app();
            let app = &scratch.0;
            crate::identity::create(app, Some("Runner".into())).unwrap();
            let initial = app.store.queued_roster().unwrap().unwrap();
            app.store.remove_outbox_roster_with_state(&initial.id, &app.state.lock().unwrap().clone(), &[]).unwrap();
            let ids: Vec<String> = (0..if case == "additions" { 5 } else { 2 }).map(|n| format!("bot{n}")).collect();
            let mut base = roster(bot(&ids[0]));
            base.bots = ids.iter().map(|id| bot(id)).collect();
            base.chats.push(ChatMeta { id: "group".into(), kind: "group".into(), title: None,
                bot_ids: ids.clone(), owner_bot_id: Some(ids[0].clone()), description: None, is_pinned: false, created_at: 1.0 });
            apply_roster(app, base.clone());
            app.store.observe_roster(0, &base).unwrap();
            let message = Message::new("group", Author::You, Body::text("preserved"));
            app.upsert_message(message.clone(), true);
            app.store.queue_outbox(&OutboxItem { id: "group-file".into(), kind: "file".into(), recipient: None,
                ciphertext: b"encrypted".to_vec(), slot: None, group: Some(crate::model::relay_name("group")) }).unwrap();
            let mut local = base.clone();
            let mut remote = base;
            if case == "additions" {
                local.chats[0].bot_ids.push("local-sixth".into());
                remote.chats[0].bot_ids.push("remote-sixth".into());
                local.bots.push(bot("local-sixth"));
                remote.bots.push(bot("remote-sixth"));
            } else {
                local.chats[0].bot_ids.remove(0);
                if case == "legacy" { remote.chats.clear(); }
                else { remote.chats[0].bot_ids.remove(1); }
            }
            let dek = app.dek().unwrap();
            app.push_slot_blob("roster", Slot::latest("roster"), None, crate::crypto::encrypt_json(&dek, "roster", &local).unwrap());
            let queued = app.store.queued_roster().unwrap().unwrap();
            if case == "legacy" { app.store.forget_queued_roster_base().unwrap(); }
            let remote_item = OutboxItem { id: "remote-roster".into(), kind: "roster".into(), recipient: None,
                ciphertext: crate::crypto::encrypt_json(&dek, "roster", &remote).unwrap(), slot: Some(Slot::latest("roster")), group: None };
            app.relay.put_blob(&url, "token", remote_item, 0).await.unwrap();
            assert_eq!(app.relay.put_blob(&url, "token", queued.clone(), 0).await.unwrap_err().status, Some(409));
            let error = preview_roster_conflict(app, &url, "token", &app.machine_file().unwrap(), &queued).await.unwrap_err();
            assert!(error.message.contains(if case == "legacy" { "lacks its original baseline" } else if case == "additions" { "7 members" } else { "0 members" }));
            assert!(pull_blobs(app, &url, "token", &app.machine_file().unwrap()).await.unwrap_err().message.contains("Roster conflict:"));
            assert!(app.message("group", &message.id).is_some());
            assert!(app.store.outbox().unwrap().iter().any(|item| item.id == "group-file"));
            assert_eq!(app.store.queued_roster().unwrap().unwrap().ciphertext, queued.ciphertext);
            let relay = slot.lock().await;
            assert_eq!(relay.0, 1);
            let ciphertext = relay.1.as_ref().unwrap()["ciphertext"].as_str().unwrap();
            let published: RosterBlob = crate::crypto::decrypt_json(&dek, "roster", &unb64(ciphertext).unwrap()).unwrap();
            assert_eq!(published.chats, remote.chats);
            task.abort();
        }
    }

    #[tokio::test]
    async fn two_offline_bots_claiming_same_dm_id_keep_local_intent_on_conflict() {
        use axum::{extract::{Query, State as HttpState}, http::StatusCode, routing::put, Json, Router};
        use serde_json::{json, Value};
        use tokio::sync::Mutex;

        async fn upload(HttpState(slot): HttpState<Arc<Mutex<(i64, Option<Value>)>>>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
            let mut slot = slot.lock().await;
            if body["expected_slot_seq"].as_i64() != Some(slot.0) {
                return (StatusCode::CONFLICT, Json(json!({"error":"Roster slot changed"})));
            }
            slot.0 += 1;
            slot.1 = Some(json!({"id":body["id"],"kind":"roster","recipient_machine_pubkey":null,
                "ciphertext":body["ciphertext"],"seq":slot.0,"created_at":1}));
            (StatusCode::OK, Json(json!({"seq":slot.0})))
        }
        async fn list(HttpState(slot): HttpState<Arc<Mutex<(i64, Option<Value>)>>>, Query(query): Query<std::collections::HashMap<String,String>>) -> Json<Value> {
            let slot = slot.lock().await;
            Json(json!({"blobs":slot.1.iter().filter(|blob| blob["seq"].as_i64().unwrap() > query["since"].parse::<i64>().unwrap()).cloned().collect::<Vec<_>>(),"seq":slot.0}))
        }
        let slot = Arc::new(Mutex::new((0, None)));
        let server = Router::new().route("/v1/blobs", put(upload).get(list)).with_state(slot.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
        let a = scratch_app();
        let b = scratch_app();
        crate::identity::create(&a.0, Some("A".into())).unwrap();
        *b.0.machine.lock().unwrap() = a.0.machine_file();
        let runner = b.0.local_device().unwrap();
        b.0.state.lock().unwrap().devices.push(runner);
        let initial = a.0.store.queued_roster().unwrap().unwrap();
        a.0.store.remove_outbox_roster_with_state(&initial.id, &a.0.state.lock().unwrap().clone(), &[]).unwrap();
        for app in [&a.0, &b.0] {
            app.store.observe_roster(0, &RosterBlob::default()).unwrap();
            let mut candidate = bot(if Arc::ptr_eq(app, &a.0) { "bot-A" } else { "bot-B" });
            candidate.runner_id = app.this_device_id().unwrap();
            candidate.capabilities.shell = false;
            app.create_bot_with_dm(candidate, Some("X".into())).unwrap();
        }
        let message = Message::new("X", Author::You, Body::text("A stays A"));
        a.0.upsert_message(message.clone(), true);
        a.0.store.queue_outbox(&OutboxItem { id: "X-file".into(), kind: "file".into(), recipient: None,
            ciphertext: b"encrypted".to_vec(), slot: None, group: Some(crate::model::relay_name("X")) }).unwrap();
        let queued = a.0.store.queued_roster().unwrap().unwrap();
        let b_roster = b.0.store.queued_roster().unwrap().unwrap();
        b.0.relay.put_blob(&url, "token", b_roster.clone(), 0).await.unwrap();
        assert_eq!(a.0.relay.put_blob(&url, "token", queued.clone(), 0).await.unwrap_err().status, Some(409));
        let local: RosterBlob = crate::crypto::decrypt_json(&a.0.dek().unwrap(), "roster", &queued.ciphertext).unwrap();
        let mut matching = local.clone();
        matching.bots.push(bot("remote-extra"));
        validate_pending_chat_identities(&a.0, &matching).unwrap();
        let merged = merge_rosters(matching, &RosterBlob::default(), &local);
        assert!(merged.bots.iter().any(|bot| bot.id == "remote-extra"));
        assert_eq!(merged.chats.iter().find(|chat| chat.id == "X").unwrap().bot_ids, ["bot-A"]);
        for _ in 0..2 {
            let error = preview_roster_conflict(&a.0, &url, "token", &a.0.machine_file().unwrap(), &queued).await.unwrap_err();
            assert!(error.message.contains("chat X"));
            assert!(pull_blobs(&a.0, &url, "token", &a.0.machine_file().unwrap()).await.unwrap_err().message.contains("chat X"));
            assert_eq!(a.0.chat("X").unwrap().meta.bot_ids, ["bot-A"]);
            assert!(a.0.bot("bot-A").is_some());
            assert!(a.0.bot("bot-B").is_none());
            assert!(a.0.message("X", &message.id).is_some());
            let outbox = a.0.store.outbox().unwrap();
            assert!(outbox.iter().any(|item| item.id == "X-file"));
            assert!(outbox.iter().any(|item| item.kind == "policy"));
            assert_eq!(a.0.store.queued_roster().unwrap().unwrap().ciphertext, queued.ciphertext);
            assert!(a.0.store.pending_chat_creates().unwrap().contains(&"X".into()));
        }
        let remote_ciphertext = slot.lock().await.1.as_ref().unwrap()["ciphertext"].as_str().unwrap().to_string();
        let remote: RosterBlob = crate::crypto::decrypt_json(&a.0.dek().unwrap(), "roster", &unb64(&remote_ciphertext).unwrap()).unwrap();
        a.0.store.observe_roster(1, &remote).unwrap();
        assert!(rebase_queued_roster(&a.0, &a.0.machine_file().unwrap(), queued.clone()).unwrap_err().message.contains("chat X"));
        assert_eq!(a.0.store.queued_roster().unwrap().unwrap().ciphertext, queued.ciphertext);
        let relay = slot.lock().await;
        assert_eq!(relay.0, 1);
        assert_eq!(unb64(relay.1.as_ref().unwrap()["ciphertext"].as_str().unwrap()).unwrap(), b_roster.ciphertext);
        task.abort();
    }

    #[test]
    fn incoming_roster_rechecks_creation_after_preflight() {
        let a = scratch_app();
        let b = scratch_app();
        crate::identity::create(&a.0, Some("A".into())).unwrap();
        *b.0.machine.lock().unwrap() = a.0.machine_file();
        let runner = b.0.local_device().unwrap();
        b.0.state.lock().unwrap().devices.push(runner);
        let initial = a.0.store.queued_roster().unwrap().unwrap();
        a.0.store.remove_outbox_roster_with_state(&initial.id, &a.0.state.lock().unwrap().clone(), &[]).unwrap();
        let mut remote_bot = bot("bot-B");
        remote_bot.runner_id = b.0.this_device_id().unwrap();
        b.0.create_bot_with_dm(remote_bot, Some("X".into())).unwrap();
        let remote = b.0.store.queued_roster().unwrap().unwrap();
        let incoming = BlobIn { id: remote.id.clone(), kind: "roster".into(), recipient_machine_pubkey: None,
            ciphertext: crate::keys::b64(&remote.ciphertext), seq: 1, created_at: 1 };
        assert!(a.0.store.queued_roster().unwrap().is_none()); // Preflight saw no local edit.
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let worker = std::thread::spawn({
            let app = a.0.clone();
            let barrier = barrier.clone();
            move || {
                barrier.wait();
                let mut local_bot = bot("bot-A");
                local_bot.runner_id = app.this_device_id().unwrap();
                local_bot.capabilities.shell = false;
                app.create_bot_with_dm(local_bot, Some("X".into())).unwrap();
                let message = Message::new("X", Author::You, Body::text("keep local"));
                app.upsert_message(message.clone(), true);
                message
            }
        });
        barrier.wait();
        let message = worker.join().unwrap();
        for remember in [false, true] {
            let error = apply_incoming_blob(&a.0, &a.0.machine_file().unwrap(), &incoming, remember).unwrap_err();
            assert!(error.message.contains("chat X"));
            assert_eq!(a.0.chat("X").unwrap().meta.bot_ids, ["bot-A"]);
            assert!(a.0.bot("bot-A").is_some() && a.0.bot("bot-B").is_none());
            assert!(a.0.message("X", &message.id).is_some());
            assert!(a.0.store.queued_roster().unwrap().is_some());
            assert!(a.0.store.outbox().unwrap().iter().any(|item| item.kind == "policy"));
            assert!(a.0.store.roster_baseline("observed").unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn first_sync_and_normal_pull_recheck_chat_created_while_fetching() {
        use axum::{extract::{Query, State as HttpState}, routing::get, Json, Router};
        use serde_json::{json, Value};
        use tokio::sync::Notify;

        struct PullGate { blob: Value, entered: Notify, release: Notify }
        async fn list(HttpState(gate): HttpState<Arc<PullGate>>,
            Query(query): Query<std::collections::HashMap<String, String>>) -> Json<Value> {
            gate.entered.notify_one();
            gate.release.notified().await;
            let since: i64 = query["since"].parse().unwrap();
            let seq = gate.blob["seq"].as_i64().unwrap();
            Json(json!({"blobs": if since < seq { vec![gate.blob.clone()] } else { Vec::new() }, "seq":seq}))
        }
        for first in [true, false] {
            let a = scratch_app();
            let b = scratch_app();
            crate::identity::create(&a.0, Some("A".into())).unwrap();
            *b.0.machine.lock().unwrap() = a.0.machine_file();
            let runner = b.0.local_device().unwrap();
            b.0.state.lock().unwrap().devices.push(runner);
            let initial = a.0.store.queued_roster().unwrap().unwrap();
            a.0.store.remove_outbox_roster_with_state(&initial.id, &a.0.state.lock().unwrap().clone(), &[]).unwrap();
            let mut remote_bot = bot("bot-B");
            remote_bot.runner_id = b.0.this_device_id().unwrap();
            b.0.create_bot_with_dm(remote_bot, Some("X".into())).unwrap();
            let remote = b.0.store.queued_roster().unwrap().unwrap();
            let seq = if first { 1 } else { 2 };
            if !first { a.0.state.lock().unwrap().last_seq = 1; }
            let gate = Arc::new(PullGate { blob: json!({"id":remote.id,"kind":"roster",
                "recipient_machine_pubkey":null,"ciphertext":crate::keys::b64(&remote.ciphertext),
                "seq":seq,"created_at":1}), entered: Notify::new(), release: Notify::new() });
            let server = Router::new().route("/v1/blobs", get(list)).with_state(gate.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server_task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
            let pulling = tokio::spawn({
                let app = a.0.clone();
                async move { pull_blobs(&app, &url, "token", &app.machine_file().unwrap()).await }
            });
            gate.entered.notified().await;
            let mut local_bot = bot("bot-A");
            local_bot.runner_id = a.0.this_device_id().unwrap();
            local_bot.capabilities.shell = false;
            a.0.create_bot_with_dm(local_bot, Some("X".into())).unwrap();
            let message = Message::new("X", Author::You, Body::text("keep local"));
            a.0.upsert_message(message.clone(), true);
            gate.release.notify_one();
            let error = pulling.await.unwrap().unwrap_err();
            assert!(error.message.contains("chat X"));
            assert_eq!(a.0.chat("X").unwrap().meta.bot_ids, ["bot-A"]);
            assert!(a.0.bot("bot-A").is_some() && a.0.bot("bot-B").is_none());
            assert!(a.0.message("X", &message.id).is_some());
            assert!(a.0.store.queued_roster().unwrap().is_some());
            assert!(a.0.store.pending_chat_creates().unwrap().contains(&"X".into()));
            server_task.abort();
        }
    }

    #[test]
    fn migrated_id_only_marker_rejects_matching_edited_foreign_group() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let mut base = roster(bot("A"));
        base.bots.push(bot("B"));
        apply_roster(app, base);
        app.create_chat(ChatMeta { id: "G".into(), kind: "group".into(), title: None,
            bot_ids: vec!["A".into()], owner_bot_id: Some("A".into()), description: None, is_pinned: false, created_at: 1.0 }).unwrap();
        app.update_chat_meta("G", |chat| chat.bot_ids.push("B".into())).unwrap();
        app.store.forget_chat_create_identity("G").unwrap();
        let mut foreign = roster(bot("A"));
        foreign.bots.push(bot("B"));
        foreign.chats.push(ChatMeta { id: "G".into(), kind: "group".into(), title: Some("Foreign".into()),
            bot_ids: vec!["A".into(), "B".into()], owner_bot_id: Some("A".into()), description: None, is_pinned: false, created_at: 2.0 });
        let before = app.chat("G").unwrap();
        let blob = BlobIn { id: "foreign-roster".into(), kind: "roster".into(), recipient_machine_pubkey: None,
            ciphertext: crate::keys::b64(&crate::crypto::encrypt_json(&app.dek().unwrap(), "roster", &foreign).unwrap()),
            seq: 1, created_at: 1 };
        assert!(apply_incoming_blob(app, &app.machine_file().unwrap(), &blob, true).unwrap_err().message.contains("older creation marker"));
        assert_eq!(app.chat("G").unwrap().meta, before.meta);
        assert!(app.store.pending_chat_creates().unwrap().contains(&"G".into()));
    }

    #[test]
    fn never_uploaded_deleted_chat_marker_clears_on_remote_omission() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let mut base = roster(bot("A"));
        base.bots.push(bot("B"));
        apply_roster(app, base.clone());
        app.store.observe_roster(0, &base).unwrap();
        app.create_chat(ChatMeta { id: "G".into(), kind: "group".into(), title: None,
            bot_ids: vec!["A".into()], owner_bot_id: Some("A".into()), description: None, is_pinned: false, created_at: 1.0 }).unwrap();
        app.delete_chat("G");
        assert!(app.store.pending_chat_creates().unwrap().contains(&"G".into()));
        let remote = BlobIn { id: "remote-empty".into(), kind: "roster".into(), recipient_machine_pubkey: None,
            ciphertext: crate::keys::b64(&crate::crypto::encrypt_json(&app.dek().unwrap(), "roster", &base).unwrap()),
            seq: 1, created_at: 1 };
        apply_incoming_blob(app, &app.machine_file().unwrap(), &remote, true).unwrap();
        assert!(app.chat("G").is_none());
        assert!(!app.store.pending_chat_creates().unwrap().contains(&"G".into()));
    }

    #[tokio::test]
    async fn lost_roster_response_recovers_original_group_identity() {
        use axum::{extract::{Query, State as HttpState}, http::StatusCode, routing::put, Json, Router};
        use serde_json::{json, Value};
        use tokio::sync::{Mutex, Notify};
        struct LostReply { roster: Option<Value>, accepted: Arc<Notify>, release: Arc<Notify> }
        async fn upload(HttpState(slot): HttpState<Arc<Mutex<LostReply>>>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
            if body["kind"] != "roster" {
                return (StatusCode::OK, Json(json!({"seq":0})));
            }
            let (accepted, release) = {
                let mut slot = slot.lock().await;
                slot.roster = Some(json!({"id":body["id"],"kind":"roster","recipient_machine_pubkey":null,
                    "ciphertext":body["ciphertext"],"seq":1,"created_at":1}));
                (slot.accepted.clone(), slot.release.clone())
            };
            accepted.notify_one();
            release.notified().await;
            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":"reply lost"})))
        }
        async fn list(HttpState(slot): HttpState<Arc<Mutex<LostReply>>>, Query(query): Query<std::collections::HashMap<String, String>>) -> Json<Value> {
            let slot = slot.lock().await;
            let since: i64 = query["since"].parse().unwrap();
            Json(json!({"blobs": if since == 0 { slot.roster.iter().cloned().collect::<Vec<_>>() } else { Vec::new() }, "seq":1}))
        }
        let slot = Arc::new(Mutex::new(LostReply { roster: None, accepted: Arc::new(Notify::new()), release: Arc::new(Notify::new()) }));
        let server = Router::new().route("/v1/blobs", put(upload).get(list)).with_state(slot.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
        for case in ["group_edit", "bot_edit", "bot_delete"] {
        slot.lock().await.roster = None;
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let initial = app.store.queued_roster().unwrap().unwrap();
        app.store.remove_outbox_roster_with_state(&initial.id, &app.state.lock().unwrap().clone(), &[]).unwrap();
        let mut base = roster(bot("A"));
        base.bots.push(bot("B"));
        apply_roster(app, base.clone());
        app.store.observe_roster(0, &base).unwrap();
        if case == "group_edit" {
            app.create_chat(ChatMeta { id: "G".into(), kind: "group".into(), title: None,
                bot_ids: vec!["A".into()], owner_bot_id: Some("A".into()), description: None, is_pinned: false, created_at: 1.0 }).unwrap();
        } else {
            let mut new_bot = bot("new-bot");
            new_bot.runner_id = app.this_device_id().unwrap();
            app.create_bot_with_dm(new_bot, Some("G".into())).unwrap();
        }
        let accepted = slot.lock().await.accepted.clone();
        let release = slot.lock().await.release.clone();
        let upload_task = tokio::spawn({ let app = app.clone(); let url = url.clone();
            async move { drain_outbox(&app, &url, "token").await } });
        accepted.notified().await;
        match case {
            "group_edit" => app.update_chat_meta("G", |chat| chat.bot_ids.push("B".into())).unwrap(),
            "bot_edit" => { app.update_bot("new-bot", |bot| bot.name = "Renamed".into()).unwrap(); }
            _ => { app.delete_bot("new-bot").unwrap(); }
        }
        release.notify_one();
        assert!(upload_task.await.unwrap().is_err());
        let queued = app.store.queued_roster().unwrap().unwrap();
        preview_roster_conflict(app, &url, "token", &app.machine_file().unwrap(), &queued).await.unwrap();
        pull_blobs(app, &url, "token", &app.machine_file().unwrap()).await.unwrap();
        assert!(app.store.pending_chat_creates().unwrap().is_empty());
        let queued = app.store.queued_roster().unwrap().unwrap();
        let result: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &queued.ciphertext).unwrap();
        match case {
            "group_edit" => assert_eq!(result.chats.iter().find(|chat| chat.id == "G").unwrap().bot_ids, ["A", "B"]),
            "bot_edit" => assert_eq!(result.bots.iter().find(|bot| bot.id == "new-bot").unwrap().name, "Renamed"),
            _ => assert!(!result.bots.iter().any(|bot| bot.id == "new-bot")),
        }
        assert_eq!(app.store.roster_baseline("queued").unwrap().unwrap().0, 1, "{case}");
        }
        task.abort();
    }

    #[tokio::test]
    async fn accepted_group_creation_with_newer_membership_edit_drains() {
        use axum::{extract::State as HttpState, http::StatusCode, routing::put, Json, Router};
        use serde_json::{json, Value};
        use tokio::sync::{Mutex, Notify};

        struct RelaySlot {
            seq: i64,
            roster: Option<Value>,
            accepted: Arc<Notify>,
            release: Arc<Notify>,
        }
        async fn upload(HttpState(slot): HttpState<Arc<Mutex<RelaySlot>>>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
            let (seq, accepted, release) = {
                let mut slot = slot.lock().await;
                if body["kind"] == "roster" && body["expected_slot_seq"].as_i64() != Some(slot.roster.as_ref().map_or(0, |previous| previous["seq"].as_i64().unwrap())) {
                    return (StatusCode::CONFLICT, Json(json!({"error":"Roster slot changed"})));
                }
                slot.seq += 1;
                let seq = slot.seq;
                let first_roster = body["kind"] == "roster" && slot.roster.is_none();
                if body["kind"] == "roster" {
                    slot.roster = Some(json!({"id":body["id"],"kind":"roster","recipient_machine_pubkey":null,
                        "ciphertext":body["ciphertext"],"seq":seq,"created_at":1}));
                }
                (seq, first_roster.then(|| slot.accepted.clone()), first_roster.then(|| slot.release.clone()))
            };
            if let (Some(accepted), Some(release)) = (accepted, release) {
                accepted.notify_one();
                release.notified().await;
            }
            (StatusCode::OK, Json(json!({"seq":seq})))
        }
        for case in ["add_member", "remove_owner", "delete", "bot_edit", "bot_delete"] {
        let slot = Arc::new(Mutex::new(RelaySlot { seq: 0, roster: None, accepted: Arc::new(Notify::new()), release: Arc::new(Notify::new()) }));
        let server = Router::new().route("/v1/blobs", put(upload)).with_state(slot.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let initial = app.store.queued_roster().unwrap().unwrap();
        app.store.remove_outbox_roster_with_state(&initial.id, &app.state.lock().unwrap().clone(), &[]).unwrap();
        app.store.acknowledge_chat_creates(&app.store.pending_chat_creates().unwrap()).unwrap();
        let mut base = roster(bot("A"));
        base.bots.push(bot("B"));
        apply_roster(app, base.clone());
        app.store.observe_roster(0, &base).unwrap();
        if case.starts_with("bot_") {
            let mut new_bot = bot("new-bot");
            new_bot.runner_id = app.this_device_id().unwrap();
            app.create_bot_with_dm(new_bot, Some("G".into())).unwrap();
        } else {
            app.create_chat(ChatMeta { id: "G".into(), kind: "group".into(), title: None,
                bot_ids: if case == "add_member" { vec!["A".into()] } else { vec!["A".into(), "B".into()] },
                owner_bot_id: Some("A".into()), description: None, is_pinned: false, created_at: 1.0 }).unwrap();
        }
        let accepted = app.store.queued_roster().unwrap().unwrap();
        let accepted_signal = slot.lock().await.accepted.clone();
        let release = slot.lock().await.release.clone();
        let draining = tokio::spawn({
            let app = app.clone();
            let url = url.clone();
            async move { drain_outbox(&app, &url, "token").await }
        });
        accepted_signal.notified().await;
        // Server accepted G. Client has not received response. Replace queued snapshot.
        match case {
            "add_member" => app.update_chat_meta("G", |chat| chat.bot_ids.push("B".into())).unwrap(),
            "remove_owner" => app.update_chat_meta("G", |chat| {
                chat.bot_ids.retain(|id| id != "A");
                chat.owner_bot_id = Some("B".into());
            }).unwrap(),
            "delete" => app.delete_chat("G"),
            "bot_edit" => { app.update_bot("new-bot", |bot| bot.name = "Renamed".into()).unwrap(); }
            _ => { app.delete_bot("new-bot").unwrap(); }
        }
        assert_ne!(accepted.id, app.store.queued_roster().unwrap().unwrap().id);
        release.notify_one();
        draining.await.unwrap().unwrap();
        assert!(app.store.pending_chat_creates().unwrap().is_empty(), "{case}: {:?}", app.store.pending_chat_creates().unwrap());
        assert!(app.store.queued_roster().unwrap().is_none());
        let published = slot.lock().await.roster.clone().unwrap();
        let final_roster: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster",
            &unb64(published["ciphertext"].as_str().unwrap()).unwrap()).unwrap();
        match case {
            "add_member" => assert_eq!(final_roster.chats.iter().find(|chat| chat.id == "G").unwrap().bot_ids, ["A", "B"]),
            "remove_owner" => {
                let chat = final_roster.chats.iter().find(|chat| chat.id == "G").unwrap();
                assert_eq!(chat.bot_ids, ["B"]);
                assert_eq!(chat.owner_bot_id.as_deref(), Some("B"));
            }
            "delete" | "bot_delete" => assert!(!final_roster.chats.iter().any(|chat| chat.id == "G")),
            _ => assert!(final_roster.chats.iter().any(|chat| chat.id == "G")),
        }
        if case == "bot_edit" {
            assert_eq!(final_roster.bots.iter().find(|bot| bot.id == "new-bot").unwrap().name, "Renamed");
        }
        if case == "bot_delete" { assert!(!final_roster.bots.iter().any(|bot| bot.id == "new-bot")); }
        assert_eq!(app.chat("G").is_some(), case != "delete" && case != "bot_delete");
        task.abort();
        }
    }

    #[test]
    fn lost_roster_reply_preserves_unrelated_remote_creations() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        app.store.acknowledge_chat_creates(&app.store.pending_chat_creates().unwrap()).unwrap();
        let base = roster(bot("A"));
        apply_roster(app, base.clone());
        app.store.observe_roster(0, &base).unwrap();
        app.store.advance_queued_roster_base().unwrap();
        app.create_chat(ChatMeta { id: "G".into(), kind: "group".into(), title: None,
            bot_ids: vec!["A".into()], owner_bot_id: Some("A".into()), description: None, is_pinned: false, created_at: 1.0 }).unwrap();
        let accepted_chat = app.chat("G").unwrap().meta;
        let submitted = app.store.queued_roster().unwrap().unwrap();
        let submitted: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &submitted.ciphertext).unwrap();
        app.store.record_submitted_roster(0, &submitted).unwrap();
        app.update_chat_meta("G", |chat| chat.title = Some("Local edit after upload".into())).unwrap();
        let reopened = App::load(Config { home: scratch.1.clone(), port: 0 }).unwrap();
        let app = &reopened;

        // Another Device publishes its own creations after accepting G, before this
        // Device recovers the lost upload response.
        let mut remote = base;
        remote.chats.push(accepted_chat);
        remote.bots.push(bot("remote-bot"));
        remote.chats.push(ChatMeta { id: "remote-group".into(), kind: "group".into(), title: None,
            bot_ids: vec!["remote-bot".into()], owner_bot_id: Some("remote-bot".into()), description: None, is_pinned: false, created_at: 2.0 });
        let blob = BlobIn { id: "other-device-roster".into(), kind: "roster".into(), recipient_machine_pubkey: None,
            ciphertext: crate::keys::b64(&crate::crypto::encrypt_json(&app.dek().unwrap(), "roster", &remote).unwrap()),
            seq: 2, created_at: 2 };
        apply_incoming_blob(app, &app.machine_file().unwrap(), &blob, true).unwrap();

        assert!(app.bot("remote-bot").is_some(), "recovery dropped another Device's new bot");
        assert!(app.chat("remote-group").is_some(), "recovery dropped another Device's new group");
        assert_eq!(app.chat("G").unwrap().meta.title.as_deref(), Some("Local edit after upload"));
        let queued = app.store.queued_roster().unwrap().unwrap();
        let recovered: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &queued.ciphertext).unwrap();
        assert!(recovered.bots.iter().any(|bot| bot.id == "remote-bot"));
        assert!(recovered.chats.iter().any(|chat| chat.id == "remote-group"));
        assert_eq!(recovered.chats.iter().find(|chat| chat.id == "G").unwrap().title.as_deref(), Some("Local edit after upload"));
    }

    #[test]
    fn lost_routine_creation_reply_preserves_later_deletion_and_remote_edits() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        app.store.acknowledge_chat_creates(&app.store.pending_chat_creates().unwrap()).unwrap();
        let base = roster(bot("A"));
        apply_roster(app, base.clone());
        app.store.observe_roster(0, &base).unwrap();
        app.store.advance_queued_roster_base().unwrap();
        let routine = crate::routines::create(app, "A", "Owned routine", "every 1d", "Never run", None, false).unwrap();
        let submitted = app.store.queued_roster().unwrap().unwrap();
        let submitted: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &submitted.ciphertext).unwrap();
        app.store.record_submitted_roster(0, &submitted).unwrap();
        crate::routines::delete(app, &routine.id).unwrap();
        assert!(app.store.pending_chat_creates().unwrap().is_empty());

        let mut remote = submitted;
        remote.bots[0].name = "Remote profile edit".into();
        let blob = BlobIn { id: "other-device-after-routine".into(), kind: "roster".into(), recipient_machine_pubkey: None,
            ciphertext: crate::keys::b64(&crate::crypto::encrypt_json(&app.dek().unwrap(), "roster", &remote).unwrap()),
            seq: 2, created_at: 2 };
        apply_incoming_blob(app, &app.machine_file().unwrap(), &blob, true).unwrap();
        assert!(app.routine(&routine.id).is_none(), "lost creation response resurrected a locally deleted routine");
        assert_eq!(app.bot("A").unwrap().name, "Remote profile edit");
        let queued = app.store.queued_roster().unwrap().unwrap();
        let recovered: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &queued.ciphertext).unwrap();
        assert!(!recovered.routines.iter().any(|entry| entry.id == routine.id));
        assert_eq!(recovered.bots[0].name, "Remote profile edit");
    }

    #[test]
    fn forgetting_identity_removes_pending_chat_ownership() {
        let scratch = scratch_app();
        crate::identity::create(&scratch.0, Some("Runner".into())).unwrap();
        assert!(!scratch.0.store.pending_chat_identities().unwrap().is_empty());
        let values = std::collections::BTreeMap::from([("private".into(), serde_json::json!("old account data"))]);
        scratch.0.store.save_codemode_writes("old-chat", "old-bot", &values, &[]).unwrap();
        scratch.0.forget_identity().unwrap();
        assert!(scratch.0.store.pending_chat_identities().unwrap().is_empty(), "forgotten account retained pending chat identities");
        assert!(scratch.0.store.codemode_values("old-chat", "old-bot").unwrap().is_empty(),
            "forgotten account retained script values");
        crate::identity::create(&scratch.0, Some("New account".into())).unwrap();
        let reopened = App::load(Config { home: scratch.1.clone(), port: 0 }).unwrap();
        assert!(reopened.store.codemode_values("old-chat", "old-bot").unwrap().is_empty());
        let current_chats: Vec<String> = reopened.state.lock().unwrap().chats.iter().map(|chat| chat.meta.id.clone()).collect();
        assert!(reopened.store.pending_chat_identities().unwrap().iter().all(|(id, _)| current_chats.contains(id)),
            "new account inherited pending ownership from the forgotten account");
    }
    /// A relay log served past `since`, restricted to the requested kinds.
    async fn serve_log(log: Vec<BlobIn>) -> (String, tokio::task::JoinHandle<()>) {
        use axum::{extract::{Query, State as HttpState}, routing::get, Json, Router};
        use serde_json::{json, Value};

        async fn list(HttpState(log): HttpState<Arc<Vec<Value>>>, Query(query): Query<std::collections::HashMap<String, String>>) -> Json<Value> {
            let since: i64 = query["since"].parse().unwrap();
            let kinds: Vec<&str> = query["kinds"].split(',').collect();
            let head = log.iter().map(|blob| blob["seq"].as_i64().unwrap()).max().unwrap_or(0);
            let blobs: Vec<Value> = log.iter()
                .filter(|blob| blob["seq"].as_i64().unwrap() > since && kinds.contains(&blob["kind"].as_str().unwrap()))
                .cloned().collect();
            Json(json!({"blobs": blobs, "seq": head}))
        }
        // The first-sync fixture has no relay-backed transcript history.
        async fn group_page() -> Json<Value> {
            Json(json!({"slots": [], "has_more": false}))
        }
        let log: Vec<Value> = log.into_iter()
            .map(|blob| json!({"id": blob.id, "kind": blob.kind, "recipient_machine_pubkey": blob.recipient_machine_pubkey,
                "ciphertext": blob.ciphertext, "seq": blob.seq, "created_at": blob.created_at}))
            .collect();
        let server = Router::new().route("/v1/blobs", get(list))
            .route("/v1/groups/{group}/blobs", get(group_page)).with_state(Arc::new(log));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        (url, tokio::spawn(async move { axum::serve(listener, server).await.unwrap() }))
    }

    fn account_blob<T: serde::Serialize>(app: &App, seq: i64, kind: &str, id: &str, contents: &T) -> BlobIn {
        BlobIn { id: id.into(), kind: kind.into(), recipient_machine_pubkey: None,
            ciphertext: crate::keys::b64(&crate::crypto::encrypt_json(&app.dek().unwrap(), kind, contents).unwrap()),
            seq, created_at: seq }
    }

    fn addressed_blob<T: serde::Serialize>(app: &App, seq: i64, kind: &str, id: &str, contents: &T) -> BlobIn {
        let machine = app.machine_file().unwrap().machine().unwrap();
        BlobIn { id: id.into(), kind: kind.into(), recipient_machine_pubkey: Some(machine.pubkey()),
            ciphertext: crate::keys::b64(&crate::crypto::seal_json(&machine.box_pubkey(), contents).unwrap()),
            seq, created_at: seq }
    }

    fn queued_turn(app: &Arc<App>) -> Job {
        let mut runner_bot = bot("sync-bot");
        runner_bot.runner_id = app.this_device_id().unwrap();
        let (_, chat) = app.create_bot_with_dm(runner_bot, Some("sync-chat".into())).unwrap();
        let mut trigger = Message::new(&chat.meta.id, Author::You, Body::text("Run this queued turn"));
        trigger.queued = true;
        app.upsert_message(trigger.clone(), false);
        Job { id: "held".into(), chat_id: chat.meta.id, bot_id: "sync-bot".into(), kind: "turn".into(),
            trigger_message_id: trigger.id, routine_id: None, check: None, requested_by: app.this_device_id().unwrap(),
            from_bot_id: None, hops: 0, round: 0, is_winding_down: false, setup: None, created_at: 1.0 }
    }

    async fn wait_for_job_finished(events: &mut tokio::sync::broadcast::Receiver<Event>, id: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if matches!(events.recv().await.unwrap(), Event::JobFinished { job_id, .. } if job_id == id) {
                    break;
                }
            }
        }).await.expect("the cancelled turn should finish");
    }

    #[tokio::test]
    async fn a_lease_holds_the_cursor_before_the_first_job_and_releasing_it_resumes_in_order() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let job = queued_turn(app);
        // Keep admitted turns behind an existing chat turn, without calling a provider.
        let lock = app.chat_lock(&job.chat_id);
        let guard = lock.lock().await;
        app.state.lock().unwrap().last_seq = 5;
        let before = Message::new(&job.chat_id, Author::System, Body::text("Before the held turn"));
        let mut after = before.clone();
        after.body = Body::text("After the held turn");
        let (url, server) = serve_log(vec![
            account_blob(app, 6, "chat", "before", &ChatBlob::Upsert { message: before.clone() }),
            addressed_blob(app, 7, "job", "held-job", &job),
            account_blob(app, 8, "chat", "after", &ChatBlob::Upsert { message: after.clone() }),
        ]).await;
        let machine_file = app.machine_file().unwrap();
        let mut events = app.events.subscribe();
        app.update.prepare(app, Duration::from_secs(60));
        pull_blobs(app, &url, "token", &machine_file).await.unwrap();
        assert_eq!(app.message(&job.chat_id, &before.id).unwrap().body, Body::text("Before the held turn"));
        assert!(!app.running_turns().iter().any(|turn| turn["job_id"] == job.id));
        assert_eq!(app.state.lock().unwrap().last_seq, 6, "the cursor stops before the held turn");
        let mut saw_before = false;
        while let Ok(event) = events.try_recv() {
            match event {
                Event::MessageAdded { message, .. } if message.id == before.id => saw_before = true,
                Event::JobStarted { job_id, .. } if job_id == job.id => panic!("the held turn started"),
                _ => {}
            }
        }
        assert!(saw_before, "the app sees the transcript change before the held turn");
        let reloaded = App::load(Config { home: scratch.1.clone(), port: 0 }).unwrap();
        assert_eq!(reloaded.message(&job.chat_id, &before.id).unwrap().body, Body::text("Before the held turn"));
        assert_eq!(reloaded.state.lock().unwrap().last_seq, 6);
        drop(reloaded);

        assert!(app.update.cancel(app));
        pull_blobs(app, &url, "token", &machine_file).await.unwrap();
        assert_eq!(app.message(&job.chat_id, &before.id).unwrap().body, Body::text("After the held turn"));
        assert_eq!(app.state.lock().unwrap().last_seq, 8);
        let (mut saw_start, mut saw_after) = (false, false);
        while let Ok(event) = events.try_recv() {
            match event {
                Event::JobStarted { job_id, .. } if job_id == job.id => {
                    assert!(!saw_after, "the turn starts before the later transcript change");
                    saw_start = true;
                }
                Event::MessageUpdated { message, .. } if message.id == after.id => {
                    assert!(saw_start, "the later transcript change cannot overtake the queued turn");
                    assert_eq!(message.body, Body::text("After the held turn"));
                    saw_after = true;
                }
                _ => {}
            }
        }
        assert!(saw_start && saw_after, "the app sees the resumed turn and its following transcript change");
        app.cancel_job(&job.id);
        drop(guard);
        wait_for_job_finished(&mut events, &job.id).await;
        server.abort();
    }

    #[tokio::test]
    async fn a_held_first_sync_keeps_the_cursor_at_zero_and_saves_what_it_applied() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let job = queued_turn(app);
        let paired = crate::keys::Machine::generate();
        let before = MachineBlob { device: Device { id: paired.pubkey(), name: "Paired laptop".into(),
            model: String::new(), os: "macos".into(), os_version: String::new(), box_pubkey: paired.box_pubkey(),
            plugins: Vec::new(), updated_at: 1 }, turns: Vec::new() };
        let mut after = before.clone();
        after.device.name = "Renamed laptop".into();
        after.device.updated_at = 3;
        let (url, server) = serve_log(vec![
            account_blob(app, 1, "machine", "before", &before),
            addressed_blob(app, 2, "job", "held-job", &job),
            account_blob(app, 3, "machine", "after", &after),
        ]).await;
        let machine_file = app.machine_file().unwrap();
        app.update.prepare(app, Duration::from_secs(60));
        pull_blobs(app, &url, "token", &machine_file).await.unwrap();
        assert_eq!(app.device(&paired.pubkey()).unwrap().name, "Paired laptop");
        assert!(!app.running_turns().iter().any(|turn| turn["job_id"] == job.id));
        assert_eq!(app.state.lock().unwrap().last_seq, 0, "a held first sync does not claim the log");

        let reloaded = App::load(Config { home: scratch.1.clone(), port: 0 }).unwrap();
        assert_eq!(reloaded.device(&paired.pubkey()).unwrap().name, "Paired laptop", "the applied Device metadata survives restart");
        assert_eq!(reloaded.state.lock().unwrap().last_seq, 0);
        // Restart drops the lease. Replay on the reopened Device must still admit the held job.
        let lock = reloaded.chat_lock(&job.chat_id);
        let guard = lock.lock().await;
        let mut events = reloaded.events.subscribe();
        pull_blobs(&reloaded, &url, "token", &reloaded.machine_file().unwrap()).await.unwrap();
        assert!(reloaded.running_turns().iter().any(|turn| turn["job_id"] == job.id));
        assert_eq!(reloaded.device(&paired.pubkey()).unwrap().name, "Renamed laptop");
        assert_eq!(reloaded.state.lock().unwrap().last_seq, 3);
        reloaded.cancel_job(&job.id);
        drop(guard);
        wait_for_job_finished(&mut events, &job.id).await;
        drop(reloaded);
        server.abort();
    }

    #[tokio::test]
    async fn while_held_only_a_running_jobs_cancel_is_applied_and_the_cursor_stays() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let job = queued_turn(app);
        let lock = app.chat_lock(&job.chat_id);
        let guard = lock.lock().await;
        let mut events = app.events.subscribe();
        let mut running_job = job.clone();
        running_job.id = "running".into();
        crate::runtime::spawn_local_job(app.clone(), running_job, None, app.update.try_admit().unwrap());
        let running = app.running_jobs.lock().unwrap().get("running").unwrap().cancel.clone();
        app.state.lock().unwrap().last_seq = 1;
        let (url, server) = serve_log(vec![
            addressed_blob(app, 2, "job", "held-job", &job),
            addressed_blob(app, 3, "job_cancel", "cancel-held", &JobCancel { job_id: job.id.clone() }),
            addressed_blob(app, 4, "job_cancel", "cancel-running", &JobCancel { job_id: "running".into() }),
        ]).await;
        let machine_file = app.machine_file().unwrap();
        app.update.prepare(app, Duration::from_secs(60));
        pull_blobs(app, &url, "token", &machine_file).await.unwrap();
        assert!(running.is_cancelled(), "Stop reaches already admitted work during the drain");
        assert!(!app.running_turns().iter().any(|turn| turn["job_id"] == job.id));
        assert!(app.message(&job.chat_id, &job.trigger_message_id).unwrap().queued);
        assert_eq!(app.state.lock().unwrap().last_seq, 1, "the control pass does not advance past the queued turn");
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, Event::JobStarted { job_id, .. } if job_id == job.id), "the queued turn remains held");
        }

        assert!(app.update.cancel(app));
        pull_blobs(app, &url, "token", &machine_file).await.unwrap();
        let held = app.running_jobs.lock().unwrap().get(&job.id).unwrap().cancel.clone();
        assert!(held.is_cancelled(), "the queued Stop was retained and reaches the turn after admission resumes");
        assert_eq!(app.state.lock().unwrap().last_seq, 4);
        drop(guard);
        tokio::time::timeout(Duration::from_secs(5), async {
            let (mut running_finished, mut held_finished) = (false, false);
            while !running_finished || !held_finished {
                if let Event::JobFinished { job_id, .. } = events.recv().await.unwrap() {
                    if job_id == "running" { running_finished = true; }
                    if job_id == job.id { held_finished = true; }
                }
            }
        }).await.expect("both stopped turns should finish");
        assert!(!app.running_turns().iter().any(|turn| turn["job_id"] == "running" || turn["job_id"] == job.id));
        assert!(app.message(&job.chat_id, &job.trigger_message_id).unwrap().queued, "neither stopped turn consumes the queued message");
        server.abort();
    }

    #[tokio::test]
    async fn while_held_a_remote_turn_this_device_waits_on_still_ends() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let job = queued_turn(app);
        let lock = app.chat_lock(&job.chat_id);
        let guard = lock.lock().await;
        // A turn this Device sealed to another Runner, waiting on its `job_result`.
        app.store.insert_sent_job(&crate::app::SentJob { id: "remote-job".into(), chat_id: job.chat_id.clone(),
            bot_id: "sync-bot".into(), routine_id: None, runner_id: "other-runner".into(), sent_at: crate::config::now_secs() }).unwrap();
        crate::runtime::resume_sent_jobs(app);
        assert!(app.running_turns().iter().any(|turn| turn["job_id"] == "remote-job"));
        app.state.lock().unwrap().last_seq = 1;
        let result = JobResult { job_id: "remote-job".into(), chat_id: job.chat_id.clone(), bot_id: "sync-bot".into(), outcome: "sent".into() };
        let (url, server) = serve_log(vec![
            addressed_blob(app, 2, "job", "held-job", &job),
            addressed_blob(app, 3, "job_result", "remote-result", &result),
        ]).await;
        let machine_file = app.machine_file().unwrap();
        let mut events = app.events.subscribe();
        app.update.prepare(app, Duration::from_secs(60));
        pull_blobs(app, &url, "token", &machine_file).await.unwrap();
        wait_for_job_finished(&mut events, "remote-job").await;
        assert!(!app.running_turns().iter().any(|turn| turn["job_id"] == "remote-job"), "the remote turn ends behind the held job");
        assert!(app.store.sent_jobs().unwrap().is_empty());
        assert_eq!(app.state.lock().unwrap().last_seq, 1, "the result does not advance past the queued turn");
        assert!(!app.running_turns().iter().any(|turn| turn["job_id"] == job.id), "the queued turn remains held");
        assert!(!app.is_paused());

        assert!(app.update.cancel(app));
        pull_blobs(app, &url, "token", &machine_file).await.unwrap();
        assert_eq!(app.state.lock().unwrap().last_seq, 3);
        assert!(app.running_turns().iter().any(|turn| turn["job_id"] == job.id), "the held turn starts once admission resumes");
        app.cancel_job(&job.id);
        drop(guard);
        wait_for_job_finished(&mut events, &job.id).await;
        server.abort();
    }

    #[tokio::test]
    async fn while_held_a_request_this_device_asked_still_hears_its_response() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let job = queued_turn(app);
        let lock = app.chat_lock(&job.chat_id);
        let _guard = lock.lock().await;
        let runner = crate::keys::Machine::generate();
        {
            let mut state = app.state.lock().unwrap();
            state.devices.push(Device { id: runner.pubkey(), name: "Other Runner".into(), model: String::new(), os: "linux".into(),
                os_version: String::new(), box_pubkey: runner.box_pubkey(), plugins: Vec::new(), updated_at: 1 });
            state.device_online.insert(runner.pubkey());
            state.last_seq = 1;
        }
        app.settings.lock().unwrap().relay_url = Some("http://127.0.0.1:1".into());
        let asking = {
            let (app, runner_id) = (app.clone(), runner.pubkey());
            tokio::spawn(async move {
                crate::requests::ask_within(&app, &runner_id, "memory.read", serde_json::json!({ "bot_id": "remote-bot" }), Duration::from_secs(10)).await
            })
        };
        // The other Runner reads the sealed request this Device queued and answers it.
        let request: Request = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(item) = app.store.outbox().unwrap().into_iter().find(|item| item.kind == "request") {
                    break crate::crypto::unseal_json(&runner.box_secret, &item.ciphertext).unwrap();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("the request should be queued for the other Runner");
        let response = Response { request_id: request.id, body: serde_json::json!({ "index": "remembered" }), error: None };
        let (url, server) = serve_log(vec![
            addressed_blob(app, 2, "job", "held-job", &job),
            addressed_blob(app, 3, "response", "remote-response", &response),
        ]).await;
        let mut events = app.events.subscribe();
        app.update.prepare(app, Duration::from_secs(60));
        pull_blobs(app, &url, "token", &app.machine_file().unwrap()).await.unwrap();
        let answer = tokio::time::timeout(Duration::from_secs(5), asking).await.expect("the request's wait ends behind the held job").unwrap();
        assert_eq!(answer, Ok(serde_json::json!({ "index": "remembered" })));
        assert_eq!(app.state.lock().unwrap().last_seq, 1, "the response does not advance past the queued turn");
        assert!(!app.running_turns().iter().any(|turn| turn["job_id"] == job.id));
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, Event::JobStarted { job_id, .. } if job_id == job.id), "the queued turn remains held");
        }
        assert!(!app.is_paused());
        server.abort();
    }

}
