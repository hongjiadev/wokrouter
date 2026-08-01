const maxRecentSuccesses = 64;

export class RecentSuccesses {
  private readonly keys = new Set<string>();
  private readonly order: string[] = [];

  has(key: string): boolean {
    return this.keys.has(key);
  }

  remember(key: string): void {
    if (this.keys.has(key)) {
      return;
    }
    this.keys.add(key);
    this.order.push(key);
    if (this.order.length > maxRecentSuccesses) {
      const oldest = this.order.shift();
      if (oldest !== undefined) {
        this.keys.delete(oldest);
      }
    }
  }
}
