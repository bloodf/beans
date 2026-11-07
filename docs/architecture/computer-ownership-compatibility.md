# Computer ownership compatibility fixture

At source revision `755ecf123564abd8ef8c9657e04e4d9b3be7d7a2`, referenced by [Computer ownership](computer-ownership.md#current-evidence-and-boundary), rustc `1.99.0 (b940084d7 2026-09-28)` (Homebrew) compiled and ran this isolated fixture on darwin 25.6.0 arm64; Bun `1.4.2` is available. Existing serde artifacts were linked, without Cargo. `Attachment`/`BotLook` are JSON substitutes for unrelated fields; `Roster` is minimal. Capabilities declaration/default, Bot declaration and bot merge macro/invocation are source-extracted, not a reimplementation. This exercises deserialization and bot field merge, not policy projection, whole-core startup or installed clients.

SHA-256 source bindings:

- `crates/cli/src/model.rs`: `a3b15eea514e058df8ad3773a86fa2cc161935cd1832d73dbd7ccb9aea7bbbce`
- `crates/cli/src/sync.rs`: `20c213aa779bc431f14dad7f6db2c05aab07df599c193677d038cb1685411b4d`
- Fixture: `f686d817c5c3a039b35d5ada8f050d9f13548d3a7f7588f138e4d6200ec7ecb2`

Write the following unchanged to `$SCRATCH/fixture.rs`; set `DEPS` to the existing matching Rust artifact directory. No build of Beans is required.

```sh
rustc --edition=2021 "$SCRATCH/fixture.rs" -L dependency="$DEPS" --extern serde="$DEPS/libserde-492747a8269cbb1d.rlib" --extern serde_json="$DEPS/libserde_json-8466cd8d0e3ab640.rlib" -o "$SCRATCH/fixture" && "$SCRATCH/fixture"
```

Raw output, exit 0:

```text
PASS missing capabilities keeps shell/write/plugin defaults
PASS nested computer capability rejects: unknown field `computer`, expected one of `shell`, `write`, `plugins`
PASS unknown bot-level computer field is discarded on typed round trip (unsafe rollout)
PASS unrelated name edit preserves remote capability restriction
PASS concurrent capability edit replaces whole capability field before policy projection; not per-key merge
```

Generate the source-extracted fixture from the pinned checkout with Python 3, then move `fixture.rs` into `$SCRATCH` before the compile command above. The generator preserves the recorded fixture bytes, including source comments.

```python
from pathlib import Path
model = Path("crates/cli/src/model.rs").read_text()
sync = Path("crates/cli/src/sync.rs").read_text()
cap = model[model.index('#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]\n#[serde(default, deny_unknown_fields)]'):model.index('\nimpl Capabilities {')]
bot = model[model.index('#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]\npub struct Bot {'):model.index('\n/// One Auto-review')]
merge = sync[sync.index('    macro_rules! merge_entities {', sync.index('fn merge_rosters(')):sync.index('    merge_entities!(chats,', sync.index('fn merge_rosters('))]
prefix = 'use serde::{Serialize,Deserialize};\nuse serde_json::{Value,json};\ntype Attachment = Value;\nmod appearance {pub type BotLook = serde_json::Value;}\n'
harness = '\n#[derive(Clone)] struct Roster {bots: Vec<Bot>}\nfn merge(mut remote:Roster,base:&Roster,queued:&Roster)->Roster {\n'
tests = 'fn main(){\n let raw=json!({"id":"bot-fixture","name":"base","description":"","symbol_name":"","accent":"","runner_id":"runner-fixture","provider":"fixture","created_at":0});\n let baseline:Bot=serde_json::from_value(raw.clone()).unwrap();\n assert_eq!(baseline.capabilities,Capabilities{shell:true,write:true,plugins:None});\n println!("PASS missing capabilities keeps shell/write/plugin defaults");\n let mut nested=raw.clone(); nested["capabilities"]=json!({"computer":true});\n let err=serde_json::from_value::<Bot>(nested).unwrap_err(); assert!(err.to_string().contains("unknown field `computer`"));\n println!("PASS nested computer capability rejects: {}",err);\n let mut top=raw; top["computer"]=json!({"version":1,"enabled":true});\n let old:Bot=serde_json::from_value(top).unwrap(); assert!(serde_json::to_value(old).unwrap().get("computer").is_none());\n println!("PASS unknown bot-level computer field is discarded on typed round trip (unsafe rollout)");\n let base=Roster{bots:vec![baseline.clone()]}; let mut remote=base.clone(); remote.bots[0].capabilities.shell=false;\n let mut local=base.clone(); local.bots[0].name="renamed".into();\n let merged=merge(remote.clone(),&base,&local);assert_eq!(merged.bots[0].name,"renamed");assert!(!merged.bots[0].capabilities.shell);\n println!("PASS unrelated name edit preserves remote capability restriction");\n local.bots[0].capabilities.write=false;let merged=merge(remote,&base,&local);assert!(merged.bots[0].capabilities.shell);assert!(!merged.bots[0].capabilities.write);\n println!("PASS concurrent capability edit replaces whole capability field before policy projection; not per-key merge");\n}\n'
fixture = prefix + cap + bot.replace('crate::appearance::BotLook', 'appearance::BotLook') + harness + merge + 'remote}\n' + tests
Path("fixture.rs").write_text(fixture)
```
