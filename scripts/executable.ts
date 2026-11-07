import { accessSync, constants, statSync } from "node:fs"
import { delimiter, join } from "node:path"

/** Locate an executable on PATH. */
export function findExecutable(command: string): string | null {
  const extensions = process.platform === "win32" ? (process.env.PATHEXT ?? ".EXE;.CMD;.BAT").split(";") : [""]
  if (!process.env.PATH) return null
  for (const directory of process.env.PATH.split(delimiter)) {
    for (const extension of extensions) {
      const candidate = join(directory, command + extension)
      try {
        if (statSync(candidate).isFile()) {
          accessSync(candidate, constants.X_OK)
          return candidate
        }
      } catch {
        continue
      }
    }
  }
  return null
}
