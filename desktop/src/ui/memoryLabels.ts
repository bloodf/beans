import { L } from "../l10n";
import type { MemoryHealth, MemoryOperationState } from "../model/memoryService";

export function memoryHealthLabel(status: MemoryHealth["status"]): string {
  switch (status) {
    case "ready": return L("Ready");
    case "degraded": return L("Degraded");
    case "setup_required": return L("Setup required");
  }
}

export function memoryOperationLabel(state: MemoryOperationState): string {
  switch (state) {
    case "queued": return L("Queued, not stored");
    case "submitted": return L("Submitted, not stored");
    case "processing": return L("Processing, not stored");
    case "completed": return L("Stored");
    case "failed": return L("Failed, not stored");
    case "delivery_unknown": return L("Delivery unknown, not confirmed stored");
  }
}
