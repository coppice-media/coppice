export const BOOK_REQUEST_STATUS_VALUES = [
	'PENDING',
	'SEARCHING',
	'AWAITING_APPROVAL',
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
	AWAITING_APPROVAL: 'Legacy: awaiting approval',
	APPROVED: 'Approved',
	SEARCHING: 'Legacy: source search',
	NEEDS_SELECTION: 'Legacy: selection needed',
	GRABBED: 'Legacy: acquisition started',
	IMPORTING: 'Legacy: importing',
	QUEUED: 'Legacy: queued',
	COMPLETED: 'Legacy: completed',
	REJECTED: 'Rejected',
	FAILED: 'Legacy: failed',
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
	GRABBED: 'secondary',
	IMPORTING: 'secondary',
	QUEUED: 'outline',
	COMPLETED: 'default',
	REJECTED: 'destructive',
	FAILED: 'destructive',
}

export const REQUEST_STATUS_DESCRIPTIONS: Record<string, string> = {
	PENDING: 'Waiting for an operator to review this request.',
	AWAITING_APPROVAL: 'Legacy approval state retained for an existing request; an operator can still record a decision.',
	APPROVED: 'An operator approved this request. Acquisition is handled outside Coppice.',
	SEARCHING: 'Legacy acquisition state retained from an earlier connector workflow.',
	NEEDS_SELECTION: 'Legacy acquisition state retained from an earlier connector workflow.',
	GRABBED: 'Legacy acquisition state retained from an earlier connector workflow.',
	IMPORTING: 'Legacy acquisition state retained from an earlier connector workflow.',
	QUEUED: 'Legacy acquisition state retained from an earlier connector workflow.',
	COMPLETED: 'Legacy acquisition state retained from an earlier connector workflow.',
	REJECTED: 'An operator rejected this request.',
	FAILED: 'Legacy acquisition state retained from an earlier connector workflow.',
}

export const APPROVAL_STATUS_LABELS: Record<string, string> = {
	PENDING: 'Pending review',
	APPROVED: 'Approved',
	REJECTED: 'Declined',
	REVOKED: 'Revoked',
	EXPIRED: 'Expired',
}

export function safeText(value: unknown, maxLength = 240): string {
	const clean = String(value ?? '')
		.replace(/[\u0000-\u001f\u007f]/g, '')
		.trim()
	return clean.length > maxLength ? `${clean.slice(0, maxLength - 1)}…` : clean
}

export function statusLabel(status: string | null | undefined): string {
	const normalized = status?.toUpperCase() ?? ''
	return (
		REQUEST_STATUS_LABELS[normalized] ??
		(normalized ? safeText(normalized.replace(/_/g, ' ')) : 'Unknown status')
	)
}

export function statusDescription(status: string | null | undefined): string {
	return (
		REQUEST_STATUS_DESCRIPTIONS[status?.toUpperCase() ?? ''] ??
		'The server recorded this request status.'
	)
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
 * Read only the allow-listed metadata handoff fields. Request creation does not
 * accept transport URLs, credentials, or arbitrary endpoints.
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
