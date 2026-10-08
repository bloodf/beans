// Native Go evaluates this bundle without a browser or network. Keep the same
// request constructors, masked readers, consent and one-use setup drafts as TS.
import * as service from "./memoryService";
import * as setup from "./memorySetup";
Object.assign(globalThis, { BeansMemory: { ...service, ...setup } });
