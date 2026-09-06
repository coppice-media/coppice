import type { JobStatus, NotificationKind } from '$lib/graphql/generated/graphql';

const DAY_MS = 86_400_000;

/** A single counted day of `readingStats.days`. */
export type ActivityDay = {
	date: string;
	sessions: number;
	minutes: number;
	pages: number;
};

export type HeatmapMetric = 'minutes' | 'pages';

export type HeatmapCell = {
	/** `YYYY-MM-DD`, or `null` for the padding days of the first/last week. */
	date: string | null;
	sessions: number;
	minutes: number;
	pages: number;
	/** `0` for no activity, `1`–`4` for rising intensity of the metric. */
	level: 0 | 1 | 2 | 3 | 4;
};

export type HeatmapColumn = {
	/** Sunday first, so a column is one calendar week. */
	cells: HeatmapCell[];
	/** Set on the first column of each month, for the axis above the grid. */
	monthLabel: string | null;
};

const MONTHS = [
	'Jan',
	'Feb',
	'Mar',
	'Apr',
	'May',
	'Jun',
	'Jul',
	'Aug',
	'Sep',
	'Oct',
	'Nov',
	'Dec'
];

/**
 * Days since the epoch for a `NaiveDate`. The server counts *logical* reading
 * days, so the grid has to be built in whole days with no timezone applied:
 * parsing `2026-09-06` as local time would shift the whole strip west of UTC.
 */
function dayNumber(date: string): number {
	const [year, month, day] = date.split('-').map(Number);
	return Date.UTC(year, month - 1, day) / DAY_MS;
}

/** `0` = Sunday. The epoch (day 0) was a Thursday, hence the offset. */
function weekday(day: number): number {
	return (day + 4) % 7;
}

/**
 * Cut points at the 40th/65th/85th percentile of the days that have activity.
 * Scaling against the single busiest day instead would push a normal reading
 * day into the faintest bucket and wash the whole year out.
 */
function thresholds(values: readonly number[]): [number, number, number] {
	const sorted = values.filter((value) => value > 0).sort((a, b) => a - b);
	if (sorted.length === 0) return [0, 0, 0];
	const at = (fraction: number) =>
		sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * fraction))];
	return [at(0.4), at(0.65), at(0.85)];
}

/**
 * Inclusive comparisons, so the busiest day always reaches level 4 even when
 * the whole year holds a handful of days and the 85th percentile *is* the
 * maximum.
 */
function level(value: number, [low, mid, high]: readonly number[]): 1 | 2 | 3 | 4 {
	if (value >= high) return 4;
	if (value >= mid) return 3;
	if (value >= low) return 2;
	return 1;
}

/**
 * A calendar strip of `weeks` whole weeks ending on the week that contains
 * `to`, filled from the sparse `days` the server returns. Days with a session
 * but no measured value still light up at level 1, because a synced session
 * with no elapsed time is activity the reader can see on their device.
 */
export function buildHeatmap(
	days: readonly ActivityDay[],
	to: string,
	metric: HeatmapMetric,
	weeks = 53
): HeatmapColumn[] {
	const byDay = new Map(days.map((day) => [dayNumber(day.date), day]));
	const cuts = thresholds(days.map((day) => day[metric]));
	const end = dayNumber(to);
	const lastColumnEnd = end + (6 - weekday(end));
	const start = lastColumnEnd - weeks * 7 + 1;

	const columns: HeatmapColumn[] = [];
	let previousMonth = -1;
	for (let column = 0; column < weeks; column += 1) {
		const cells: HeatmapCell[] = [];
		for (let row = 0; row < 7; row += 1) {
			const day = start + column * 7 + row;
			const activity = day <= end ? byDay.get(day) : undefined;
			cells.push({
				date: day <= end ? new Date(day * DAY_MS).toISOString().slice(0, 10) : null,
				sessions: activity?.sessions ?? 0,
				minutes: activity?.minutes ?? 0,
				pages: activity?.pages ?? 0,
				level: activity && activity.sessions > 0 ? level(activity[metric], cuts) : 0
			});
		}
		const month = new Date((start + column * 7) * DAY_MS).getUTCMonth();
		const labelled = month !== previousMonth;
		previousMonth = month;
		columns.push({ cells, monthLabel: labelled ? MONTHS[month] : null });
	}
	return columns;
}

/** `2026-09-06` as `6 Sep 2026`, with no timezone shift. */
export function dayLabel(date: string): string {
	const parsed = new Date(dayNumber(date) * DAY_MS);
	return `${parsed.getUTCDate()} ${MONTHS[parsed.getUTCMonth()]} ${parsed.getUTCFullYear()}`;
}

export const HEATMAP_LEVEL_CLASSES: Record<HeatmapCell['level'], string> = {
	0: 'bg-muted',
	1: 'bg-primary/35',
	2: 'bg-primary/55',
	3: 'bg-primary/75',
	4: 'bg-primary'
};

/**
 * What the `readEvents` stream has reported about one job since the page
 * loaded. `status` is nullable on `JobUpdate`, so a consumer falls back to the
 * persisted status from the `jobs` query.
 */
export type LiveJobProgress = {
	status: JobStatus | null;
	message: string | null;
	subtitle: string | null;
	completedTasks: number | null;
	remainingTasks: number | null;
};

export const JOB_STATUS_VARIANTS: Record<
	JobStatus,
	'default' | 'secondary' | 'destructive' | 'outline'
> = {
	RUNNING: 'default',
	QUEUED: 'secondary',
	PAUSED: 'secondary',
	COMPLETED: 'outline',
	CANCELLED: 'outline',
	FAILED: 'destructive'
};

/** Jobs the queue is still working on, as opposed to finished history. */
export const ACTIVE_JOB_STATUSES: JobStatus[] = ['RUNNING', 'QUEUED', 'PAUSED'];

/** `provider_source_health` as `Provider source health`. */
export function jobLabel(name: string): string {
	const words = name.replace(/[_-]+/g, ' ').trim();
	return words ? words[0].toUpperCase() + words.slice(1) : name;
}

export function durationLabel(ms: number): string {
	if (ms < 1000) return `${ms}ms`;
	const seconds = Math.round(ms / 1000);
	if (seconds < 60) return `${seconds}s`;
	const minutes = Math.floor(seconds / 60);
	const rest = seconds % 60;
	if (minutes < 60) return rest ? `${minutes}m ${rest}s` : `${minutes}m`;
	const hours = Math.floor(minutes / 60);
	return `${hours}h ${minutes % 60}m`;
}

/**
 * The routable notification kinds, in the order the settings matrix lists
 * them. `administrative` mirrors `NotificationKind::is_administrative` in
 * `crates/notify`: only the server owner may route those, so the switch is
 * disabled for everyone else instead of failing the mutation. `TEST` is
 * excluded because the server never routes it; it is only sent by
 * `testNotificationChannel`.
 */
export const NOTIFICATION_KINDS: {
	kind: NotificationKind;
	label: string;
	description: string;
	administrative: boolean;
}[] = [
	{
		kind: 'DEVICE_PAIRED',
		label: 'Device paired',
		description: 'A pairing was approved and the device received its credential',
		administrative: false
	},
	{
		kind: 'DEVICE_FIRST_SEEN',
		label: 'Device first seen',
		description: 'A registered device authenticated for the first time',
		administrative: false
	},
	{
		kind: 'SCAN_FINISHED',
		label: 'Scan finished',
		description: 'A library scan completed',
		administrative: true
	},
	{
		kind: 'INGEST_AWAITING_REVIEW',
		label: 'Ingest awaiting review',
		description: 'A staged item finished analysis and needs a human decision',
		administrative: true
	},
	{
		kind: 'QUALITY_FAILED',
		label: 'Quality check failed',
		description: 'A quality report contains at least one failed check',
		administrative: true
	},
	{
		kind: 'ANALYSIS_JOB_FAILED',
		label: 'Analysis job failed',
		description: 'A staged analysis job failed',
		administrative: true
	},
	{
		kind: 'PROVIDER_MATCH_DONE',
		label: 'Provider match done',
		description: 'Provider lookup finished for a staged item',
		administrative: true
	}
];

/** The wildcard `event_kind` the server accepts in place of a single kind. */
export const ALL_EVENT_KINDS = '*';
