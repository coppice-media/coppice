/** `3m ago` / `in 3m` style distance between `value` and `now`. */
export function relativeTime(value: string | null | undefined, now: Date = new Date()): string {
	if (!value) return 'never';
	const then = new Date(value);
	if (Number.isNaN(then.getTime())) return 'unknown';
	const seconds = Math.round((now.getTime() - then.getTime()) / 1000);
	if (Math.abs(seconds) < 45) return 'just now';
	const future = seconds < 0;
	const distance = (amount: number, unit: string) =>
		future ? `in ${amount}${unit}` : `${amount}${unit} ago`;
	const minutes = Math.round(Math.abs(seconds) / 60);
	if (minutes < 60) return distance(minutes, 'm');
	const hours = Math.round(minutes / 60);
	if (hours < 24) return distance(hours, 'h');
	const days = Math.round(hours / 24);
	if (days < 30) return distance(days, 'd');
	const months = Math.round(days / 30);
	if (months < 12) return distance(months, 'mo');
	return distance(Math.round(months / 12), 'y');
}

export function absoluteTime(value: string | null | undefined): string {
	if (!value) return '—';
	const date = new Date(value);
	if (Number.isNaN(date.getTime())) return value;
	return date.toLocaleString();
}

/**
 * Compact, human-readable rendering of the protocol-specific
 * `lastSyncSummary` JSON blob.
 */
export function summarizeSync(summary: unknown): string | null {
	if (summary === null || summary === undefined) return null;
	if (typeof summary === 'string') return summary || null;
	if (typeof summary !== 'object') return String(summary);
	const parts = Object.entries(summary)
		.filter(([, value]) => value !== null && value !== undefined)
		.map(([key, value]) => {
			if (typeof value === 'boolean') return `${key}: ${value ? 'yes' : 'no'}`;
			return `${key}: ${String(value)}`;
		});
	return parts.length ? parts.join(', ') : null;
}

export function minutesLabel(minutes: number): string {
	if (minutes < 60) return `${minutes}m`;
	const hours = Math.floor(minutes / 60);
	const rest = minutes % 60;
	return rest ? `${hours}h ${rest}m` : `${hours}h`;
}
