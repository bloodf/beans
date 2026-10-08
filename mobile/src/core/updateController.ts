export interface UpdateOffer { id: string; version: string; notes: string }
interface Transport { check(): Promise<UpdateOffer | null>; install(id: string): Promise<void>; cancel(): void }
export class UpdateController {
  offer: UpdateOffer | null = null;
  busy = false;
  private generation = 0;
  constructor(private readonly transport: Transport) {}
  async check(): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    const generation = ++this.generation;
    this.offer = null;
    try {
      const offer = await this.transport.check();
      if (generation === this.generation) this.offer = offer;
    } finally { this.busy = false; }
  }
  dismiss(): void { this.generation++; this.offer = null; this.transport.cancel(); }
  async install(id: string, hasDrafts: boolean): Promise<void> {
    if (this.busy || this.offer?.id !== id) throw new Error("Consent expired; check again");
    if (hasDrafts) throw new Error("Save or send drafts before installing");
    this.offer = null;
    this.busy = true;
    try { await this.transport.install(id); } finally { this.busy = false; }
  }
}
