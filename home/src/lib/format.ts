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

/**
 * `12:34` / `1:02:03` — a moment in an audiobook (milliseconds from the start
 * of the publication) the way a listener reads it. Truncated, so it never
 * claims a second the moment has not reached.
 */
export function clockLabel(positionMs: number): string {
	const total = Math.max(0, Math.floor(positionMs / 1000));
	const hours = Math.floor(total / 3600);
	const minutes = Math.floor((total % 3600) / 60);
	const seconds = String(total % 60).padStart(2, '0');
	return hours > 0
		? `${hours}:${String(minutes).padStart(2, '0')}:${seconds}`
		: `${minutes}:${seconds}`;
}

/** `1.4 GB` style size, in the SI units the server reports bytes in. */
export function bytesLabel(bytes: number): string {
	if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
	const units = ['B', 'KB', 'MB', 'GB', 'TB'];
	const exponent = Math.min(units.length - 1, Math.floor(Math.log10(bytes) / 3));
	const value = bytes / 1000 ** exponent;
	return `${exponent === 0 ? value : value.toFixed(value < 10 ? 1 : 0)} ${units[exponent]}`;
}

/** `1,204` — thousands separated, for counts in tables and cards. */
export function countLabel(count: number): string {
	return count.toLocaleString();
}

/** `1 book` / `1,204 books` — a count with the noun that matches it. */
export function countNoun(count: number, singular: string, plural = `${singular}s`): string {
	return `${count.toLocaleString()} ${count === 1 ? singular : plural}`;
}

const NAMED_ENTITIES: Record<string, string> = {
	amp: '&',
	lt: '<',
	gt: '>',
	quot: '"',
	apos: "'",
	nbsp: '\u00a0'
};

/**
 * `Alice&#039;s Adventures` → `Alice's Adventures`. Tracker titles arrive
 * HTML-escaped; decoding them into a plain string keeps the template's text
 * interpolation (never `{@html}`) as the only rendering path.
 */
export function decodeEntities(value: string): string {
	return value.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (match, entity: string) => {
		if (entity[0] !== '#') return NAMED_ENTITIES[entity.toLowerCase()] ?? match;
		const hex = entity[1] === 'x' || entity[1] === 'X';
		const code = Number.parseInt(entity.slice(hex ? 2 : 1), hex ? 16 : 10);
		return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : match;
	});
}
