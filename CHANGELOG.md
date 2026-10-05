# Changelog

The Mac app's release notes. `bun run release-mac` attaches a version's section to its update, and
Sparkle shows it in the update window. The Windows and Linux app's are in
[desktop/CHANGELOG.md](desktop/CHANGELOG.md).

## [Unreleased]

- Group inspectors offer Make Owner, and `lorca chats list` / `chats set-owner` expose the same group ownership from the terminal.
- Lossless PNG optimizations reduce bundled image sizes without changing their decoded pixels.
- MCP CLI import and health checks preserve failure exit codes; diagnostic views mask short secret values as well as long ones.
- MCP resources keep unsupported image formats available as private files instead of dropping their bytes.
- MCP list waits for an enabled server's in-progress connection before reporting health; invalid enabled servers still fail the check.
- Mac release builds can bundle a configured relay through `LORCA_DEFAULT_RELAY_URL` without embedding deployment addresses in source; development builds omit that fallback.
- Phone provider refresh keeps omitted picker catalogs for the same account and relay without replacing selected models; account and relay changes discard old catalog metadata.
- Bots show deterministic, offline Blobatar portraits across Mac, phone, Windows, and Linux; an encrypted photo still overrides each portrait.
- Pause stops bot turns and routines account-wide after Devices sync. Each bot can restrict shell, built-in file writes, and plugins; shell and allowed plugins remain capable of writing files.
- Bots can propose a draft for review. Approve & save writes it to a new workspace file only after confirmation; Decline writes nothing.
- Custom providers discover new models automatically and offer Refresh Models in Settings without replacing a bot's chosen model.
- The local websocket rejects foreign browser origins and hostnames. Protocol-3 relays reconcile encrypted bot policies and guard concurrent roster writes; upgrade the relay and paired Devices together.
- Recovering a lost roster upload response preserves other Devices' newly created bots and chats alongside newer local edits, including after restarting the Device. A routine deleted after its creation upload stays deleted even when that upload's reply is lost.
- Shell mutations and opaque programs require Auto-review even in bot workspaces; read-only commands still run automatically. File-writing flags and executable filter programs no longer inherit a read-only classification.
- Allow on a pending shell command card resumes its command on the assigned Runner; stale, duplicate, wrong-chat, and wrong-Runner answers are rejected.
- Unpairing this Device also clears pending chat ownership and stored script values before another identity uses its data directory.
- Add MCP servers from Settings or `lorca mcp` using this Runner's `mcp.json`; manage their tools, sign-in, and access through the existing review controls. Bots can select a teammate's provider, model, and supported thinking level with `edit_bot`.
- Built-in models and marketplace entries refresh from the selected Beans relay's public, versioned catalogs without sending account credentials to an upstream service. Bundled catalogs remain available offline. The Runner's `lorca` command is available to bots with shell access; changes still go through Auto-review.
- Codemode scripts can run reviewed commands on the bot's Runner, edit files, read MCP resources, and request a plugin's complete server instructions; their command output includes an exit code, and the working row names the command. Shell, file, and plugin restrictions still apply to each nested call.
- Bots on ChatGPT, Grok, and OpenCode's GPT, Grok, and Muse Spark models see the images their tools return, such as a browser plugin's screenshot or an image file they read. They used to get only the text beside the image.
- Contributor instructions use the current phone scripts and separate Runner and phone-core checks. The source-distributed JavaScriptCore avatar bundle is included in clean checkouts.
- Pairing a Mac or restoring your identity no longer asks you to connect a provider your account already has. Onboarding waits until the account's providers arrive from the relay, which a slow connection or a long list of chats used to outlast.
- Onboarding's last step says the Mac is paired, or that your identity is restored, instead of calling it your first Device.
- Pair a Device works on every paired Mac. On a Mac that had joined by pairing it showed an error, since only the Mac that created or restored your identity could pair others. If you run your own relay, update it first.
- Settings › Devices shows a machine that is paired to your account but never sent its name or system as Unknown Device, with a note to unpair it if you don't recognize it.
- While onboarding pairs or restores, the Pair or Restore button and the field are disabled beside a spinner, and Back stops a pairing that is still waiting on the other computer.
- The marketplace adds plugins for 飞书, 飞书项目, 滴答清单, 腾讯文档, 秘塔 AI 搜索, 知乎, 高德地图, and 可灵. 飞书, 飞书项目, 滴答清单, 腾讯文档, and 可灵 sign in with your account in the browser; 秘塔 AI 搜索 and 知乎 take an API key from their sites, and 高德地图 a Web Service key from the Amap console.
- Settings › Devices can unpair this Mac too: Lorca forgets the account's keys, credentials, and chats here and goes back to onboarding. When this Mac holds your identity, the confirmation says that your backup phrase becomes the only way to restore it.

## [0.1.8]

- A routine can watch for something without spending a turn each time: the bot gives it a check, a short script that looks at an inbox, a repository, or a feed at each due time and starts the bot only when it finds something new. Checks only read and never change anything. A check that finds nothing runs no turn, so it spends nothing on the bot's model; one that asks the small model to sort or screen what it read pays for those calls, which count in the chat's Spent figure. A routine's sheet shows its check, and the inspector its next check.
- When a long chat fills a bot's context, the summary that replaces the older part, and the memory save before it, reuse the prompt cache of the bot's own turn instead of sending the whole chat again, so compacting while a bot works costs a fraction of what it did.
- A bot's command shows in the chat as a card only while it needs you: Auto-review's question, with Allow once, Always allow, and Deny, or a command the bot left running for you, with its latest output in a block that scrolls.
- Running tasks: while a chat's bots run commands, a terminal button sits in the chat's toolbar, with a badge counting them when there are two or more. It lists each command with what it does, who runs it and for how long, its latest output, and Stop, which ends that command alone while the bot carries on.
- A bot's command that asks for input (a `sudo` password, an `ssh` passphrase, a `[Y/n]`) no longer hangs its turn. You answer or stop it from its card on any of your Devices. What you type goes straight to the command and is never saved in the chat, and once the command finishes, the bot hears how it went and carries on. A command that prints nothing for 20 seconds waits the same way, and one still waiting stops after 30 minutes without output, when Lorca quits, or when its chat is deleted.
- Command output reaches the bot without colors and other terminal codes, which stay in the full-output file.
- A bot's read-only command that redirects between output streams (`2>&1`, `>&2`, `&>/dev/null`) runs at once, as other read-only commands do, instead of waiting on Auto-review, so a routine runs it with nobody watching. One that writes a file through `>&`, or from inside a quoted `$(…)`, goes to Auto-review.
- Chats with ChatGPT, Grok, and OpenCode's GPT and Grok models keep reaching the provider server that holds their prompt cache, so long chats answer sooner and cost less.
- A chat's Spent figure no longer counts cached input twice on ChatGPT, Grok, and most OpenCode models.
- Claude chats reuse their prompt cache from one turn to the next in groups and after turns with many tool calls.
- A bot speaking in one of its chats no longer costs its other chats their prompt cache.
- Auto-review weighs what a bot's action could break against what you asked for, reading the chat around your request. A step your request calls for runs without asking, destructive ones included: deleting build output, stopping a process the bot started, pushing the branch when you asked for a PR, or posting a comment after you answered "yes" to the bot's question. It still asks before harm you did not ask for, such as deleting your files, discarding uncommitted work, or deploying. The rule Always allow adds names the kind of work ("deploy Railway services to production") rather than one folder or file, so it covers the next time too.
- Bots use plugins by writing a short script that calls the plugin's tools, pages through the results, and keeps only what matters, so a long list or a big search no longer fills the chat's context. A script can also have a small, fast model rate or sort many items one by one, and that cost counts in the chat's Spent figure. A change a script wants to make still asks first on a card, and saying no stops the script.
- A plugin's tools are ready from the first chat after you install it, and chats that use plugins keep their prompt cache for the whole turn. Claude bots keep their thinking when they use a plugin.
- Lorca opens on your last chat, even when you quit it with Settings open.
- Your phone still gets the notification for a reply that finishes while the relay restarts or is briefly out of reach.

## [0.1.0]

- The first release.
