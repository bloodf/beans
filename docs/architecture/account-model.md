# Account model

Entities, plaintext/ciphertext ownership and cross-Runner job routing. Identity derivation and pairing are in [Identity](identity.md); request/blob mechanics are in [Protocols](protocols.md).

Plaintext lives **on Devices**:

```
Identity 1──* Device
Device   1──* Bot          (only a Runner: os is macos, linux, or windows)
Identity 1──* Chat
Chat     *──* Bot          (kind dm: exactly 1 bot, fixed · kind group: 1–6 bots, members change)
Chat     1──* Message
Bot      1──* Routine      (a scheduled task, run in the bot's DM on its Runner)
Device   1──* Plugin       (an MCP server installed on a Runner or in its mcp.json, for every bot there)
Bot      1──* Job          (a turn on the bot's Runner)
```

| Entity             | Device                                                    | Relay                                                  |
| ------------------ | --------------------------------------------------------- | ------------------------------------------------------ |
| Identity           | Master + content + signing keys                           | Public key                                             |
| Device             | Machine keypair, `os`                                     | Machine public key + encrypted metadata blob           |
| Bot                | Decrypted profile                                         | Inside encrypted roster blobs                          |
| Routine            | Name, schedule, prompt, state                             | Inside encrypted roster blobs                          |
| Plugin             | Manifest, variables, secrets, tokens on the Runner        | Id and state inside the Runner's encrypted machine blob |
| ProviderCredential | `credentials.json` on every Device                        | Inside the encrypted `credentials` blob                |
| Chat / Message     | Account/chat DEK                                          | Encrypted blobs                                        |
| Job                | Any paired Device may create; the assigned Runner runs it | Sealed envelope to that Runner’s machine box key; deleted once run. A hard Stop sends `job_cancel` to the Device running it; the Runner seals how the turn ended (`job_result`) to the requesting Device, and lists the turn and what it is doing in its `machine` blob for every Device |

A bot's deterministic Blobatar is seeded by its stable `bot.id`. The core stores optional generated `look` settings in the encrypted roster, independently of the photo attachment in `avatar`; fresh Beans roster writers require protocol 5 and format `beans-v2`. See [Bot avatars](avatars.md) for the shared contract, API, photos, persistence, and compatibility requirements.

Creating a bot for Runner B from Device A: A writes an encrypted bot profile into the roster (paired Devices can read it) and pins B’s machine id. Bot create rejects a target whose `os` is not desktop. Turns are job envelopes addressed to B. B decrypts the job, runs the loop with the account’s provider credentials, and uploads encrypted replies.

If B is offline, the envelope waits on the relay until B fetches it. The UI infers that from decrypted roster state. A turn for a provider the account has not connected ends with a notice in the chat that says to connect it in Settings.
