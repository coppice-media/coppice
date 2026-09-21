export const BOOK_REQUEST_STATUS_VALUES = [
	'PENDING',
	'AWAITING_APPROVAL',
	'SEARCHING',
	'NEEDS_SELECTION',
	'APPROVED',
	'GRABBED',
	'IMPORTING',
	'QUEUED',
	'COMPLETED',
	'REJECTED',
	'FAILED',
] as const
export const REQUEST_STATUS_LABELS: Record<string, string> = {
	PENDING: 'Awaiting approval',
	AWAITING_APPROVAL: 'Awaiting approval',
	APPROVED: 'Approved',
	SEARCHING: 'Searching sources',
	NEEDS_SELECTION: 'Choose a release',
	RELEASE_SELECTED: 'Release selected',
	GRABBED: 'Sending to download',
	GRABBING: 'Sending to download',
	DOWNLOADING: 'Downloading',
	IMPORTING: 'Importing',
	QUEUED: 'Queued',
	COMPLETED: 'Available',
	FULFILLED: 'Available',
	REJECTED: 'Declined',
	FAILED: 'Needs attention',
	CANCELLED: 'Cancelled',
	EXPIRED: 'Expired',
}

export const REQUEST_STATUS_VARIANTS: Record<
	string,
	'default' | 'secondary' | 'outline' | 'destructive'
> = {
	PENDING: 'outline',
	AWAITING_APPROVAL: 'outline',
	APPROVED: 'secondary',
	SEARCHING: 'secondary',
	NEEDS_SELECTION: 'outline',
	RELEASE_SELECTED: 'secondary',
	GRABBED: 'secondary',
	GRABBING: 'secondary',
	DOWNLOADING: 'secondary',
	IMPORTING: 'secondary',
	QUEUED: 'outline',
	COMPLETED: 'default',
	FULFILLED: 'default',
	REJECTED: 'destructive',
	FAILED: 'destructive',
	CANCELLED: 'destructive',
	EXPIRED: 'destructive',
}

export const REQUEST_STATUS_DESCRIPTIONS: Record<string, string> = {
	PENDING: 'An operator must approve this request before source search can start.',
	AWAITING_APPROVAL: 'An operator must approve this request before source search can start.',
	APPROVED: 'The request is approved and ready to search configured sources.',
	SEARCHING: 'Configured sources are being searched for safe, usable releases.',
	NEEDS_SELECTION:
		'The gateway found multiple viable releases. Choose one before the download starts.',
	RELEASE_SELECTED: 'A release is selected. Confirm the download before it starts.',
	GRABBED: 'The private gateway accepted the release and is tracking the download.',
	GRABBING: 'The gateway accepted the release and is handing it to the downloader.',
	DOWNLOADING: 'The selected release is downloading in the private gateway.',
	IMPORTING: 'The downloaded files are being checked and staged for import.',
	QUEUED: 'The imported book is queued for the requested destination.',
	COMPLETED: 'The book was imported and is available in the requested destination.',
	FULFILLED: 'The book was imported and is available in the requested destination.',
	REJECTED: 'An operator declined this request.',
	FAILED: 'The request stopped with an actionable error. Retry when available.',
	CANCELLED: 'This request was cancelled and cannot continue.',
	EXPIRED: 'The request expired before a release could be imported.',
}

export const APPROVAL_STATUS_LABELS: Record<string, string> = {
	PENDING: 'Pending review',
	APPROVED: 'Approved',
	REJECTED: 'Declined',
	REVOKED: 'Revoked',
	EXPIRED: 'Expired',
}

export const GATEWAY_HEALTH_LABELS: Record<string, string> = {
	HEALTHY: 'Healthy',
	OK: 'Healthy',
	DEGRADED: 'Degraded',
	UNAVAILABLE: 'Unavailable',
	UNHEALTHY: 'Unavailable',
	NOT_CONFIGURED: 'Not configured',
	UNKNOWN: 'Unknown',
}

export type ReleaseLike = {
	id?: string | null
	name?: string | null
	title?: string | null
	format?: string | null
	quality?: string | null
	sizeBytes?: number | null
	size?: number | null
	seeders?: number | null
	peers?: number | null
	availability?: string | null
	score?: number | null
	scoreReasons?: readonly string[] | null
	reasons?: readonly string[] | null
	scoreComponents?: unknown
}

/**
 * A release score is presentation-only. The server remains authoritative for
 * candidates and selection; this stable tie-breaker keeps rows predictable when
 * two candidates have equal scores or arrive in a different order.
 */
export function releaseScore(release: ReleaseLike): number {
	if (typeof release.score === 'number' && Number.isFinite(release.score)) return release.score
	const quality = (release.quality ?? '').toLowerCase()
	const format = (release.format ?? '').toLowerCase()
	const availability = (release.availability ?? '').toLowerCase()
	const qualityScore =
		quality.includes('lossless') || quality.includes('flac')
			? 20
			: quality.includes('high')
				? 12
				: 0
	const formatScore =
		format.includes('epub') || format.includes('mobi') || format.includes('pdf') ? 8 : 0
	const seeders = Math.max(0, release.seeders ?? 0)
	const availabilityScore = availability === 'freeleech' ? 8 : availability === 'available' ? 4 : 0
	return Math.min(99, qualityScore + formatScore + Math.min(20, seeders) + availabilityScore)
}

export function releaseReasons(release: ReleaseLike): string[] {
	const supplied = release.scoreReasons ?? release.reasons
	if (supplied?.length) return supplied.map((reason) => safeText(reason, 120)).filter(Boolean)
	const reasons: string[] = []
	const components = release.scoreComponents
	if (components && typeof components === 'object' && !Array.isArray(components)) {
		for (const [key, value] of Object.entries(components)) {
			if (typeof value !== 'number' || value <= 0) continue
			const label = key.replace(/[A-Z]/g, (letter) => ` ${letter.toLowerCase()}`).trim()
			reasons.push(`${label} +${value}`)
		}
	}
	if (reasons.length) return reasons.slice(0, 4)
	const format = release.format?.trim()
	const quality = release.quality?.trim()
	if (format) reasons.push(format.toUpperCase())
	if (quality) reasons.push(quality)
	if ((release.seeders ?? 0) > 0)
		reasons.push(`${release.seeders} seeder${release.seeders === 1 ? '' : 's'}`)
	if ((release.availability ?? '').toLowerCase() === 'freeleech') reasons.push('freeleech')
	return reasons.length ? reasons : ['Matches the requested title']
}

export function releaseSortKey(release: ReleaseLike): string {
	return `${safeText(release.name ?? release.title, 240).toLocaleLowerCase()}\u0000${release.id ?? ''}`
}

export function sortReleases<T extends ReleaseLike>(releases: readonly T[]): T[] {
	return [...releases].sort((left, right) => {
		const score = releaseScore(right) - releaseScore(left)
		if (score) return score
		return releaseSortKey(left).localeCompare(releaseSortKey(right))
	})
}

/** Keep provider-controlled filenames from becoming paths or executable UI. */
export function safeFilePreview(value: string | null | undefined, maxLength = 180): string {
	const clean =
		(value ?? '')
			.replace(/[\u0000-\u001f\u007f]/g, '')
			.replaceAll('\\', '/')
			.split('/')
			.pop()
			?.trim() ?? ''
	if (!clean) return 'Filename hidden until the gateway verifies the download.'
	return clean.length > maxLength ? `${clean.slice(0, maxLength - 1)}…` : clean
}

export function safeText(value: unknown, maxLength = 240): string {
	const clean = String(value ?? '')
		.replace(/[\u0000-\u001f\u007f]/g, '')
		.trim()
	return clean.length > maxLength ? `${clean.slice(0, maxLength - 1)}…` : clean
}

export function formatBytes(value: number | null | undefined): string {
	if (!Number.isFinite(value) || !value || value < 1) return 'Size unavailable'
	const units = ['B', 'KB', 'MB', 'GB', 'TB']
	const exponent = Math.min(units.length - 1, Math.floor(Math.log(value) / Math.log(1000)))
	const amount = value / 1000 ** exponent
	return `${amount.toFixed(amount >= 10 || exponent === 0 ? 0 : 1)} ${units[exponent]}`
}

export function statusLabel(status: string | null | undefined): string {
	const normalized = status?.toUpperCase() ?? ''
	return (
		REQUEST_STATUS_LABELS[normalized] ??
		(normalized ? safeText(normalized.replaceAll('_', ' ')) : 'Unknown status')
	)
}

export function statusDescription(status: string | null | undefined): string {
	return (
		REQUEST_STATUS_DESCRIPTIONS[status?.toUpperCase() ?? ''] ??
		'The server will report the next step here.'
	)
}

export function gatewayHealthLabel(status: string | null | undefined): string {
	const normalized = status?.toUpperCase() ?? 'UNKNOWN'
	return GATEWAY_HEALTH_LABELS[normalized] ?? safeText(normalized.replaceAll('_', ' '))
}

export function queryText(params: URLSearchParams, key: string): string {
	return safeText(params.get(key), 240)
}

export type ExternalWorkReference = {
	sourceProvider: string
	remoteId: string
	externalKey?: string
	title: string
	authors?: string
	coverUrl?: string
}

/**
 * Read only the allow-listed handoff fields. In particular, this intentionally
 * does not accept a tracker URL, cookie, magnet, or arbitrary endpoint.
 */
export function externalReferenceFromParams(params: URLSearchParams): ExternalWorkReference | null {
	const sourceProvider = queryText(params, 'provider') || queryText(params, 'sourceProvider')
	const remoteId = queryText(params, 'remoteId')
	const title = queryText(params, 'title')
	if (!sourceProvider || !remoteId || !title) return null
	return {
		sourceProvider,
		remoteId,
		externalKey: queryText(params, 'externalKey') || undefined,
		title,
		authors: queryText(params, 'authors') || undefined,
		coverUrl: safeCoverUrl(params.get('coverUrl')),
	}
}

export function safeCoverUrl(value: string | null | undefined): string | undefined {
	if (!value) return undefined
	try {
		const url = new URL(value)
		if (url.protocol !== 'https:' && url.protocol !== 'http:') return undefined
		return url.toString()
	} catch {
		return undefined
	}
}
