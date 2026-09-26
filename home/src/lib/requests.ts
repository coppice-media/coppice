import { providerLabel as sharedProviderLabel } from '@stump/ui/editor-fields'
import type { RequestFormat } from '$lib/graphql/generated/graphql'

/** The statuses the server still assigns; everything else is a historical row. */
export const LIVE_REQUEST_STATUSES = ['PENDING', 'APPROVED', 'REJECTED'] as const

/** Statuses left behind by the retired connector workflow; shown as one `Historical` state. */
const LEGACY_REQUEST_STATUSES = [
	'SEARCHING',
	'AWAITING_APPROVAL',
	'NEEDS_SELECTION',
	'GRABBED',
	'IMPORTING',
	'QUEUED',
	'COMPLETED',
	'FAILED',
] as const

export const BOOK_REQUEST_STATUS_VALUES = [...LIVE_REQUEST_STATUSES, ...LEGACY_REQUEST_STATUSES] as const

const LEGACY: Record<string, true> = Object.fromEntries(
	LEGACY_REQUEST_STATUSES.map((status) => [status, true] as const)
)

export const REQUEST_STATUS_LABELS: Record<string, string> = {
	PENDING: 'Awaiting approval',
	APPROVED: 'Approved',
	REJECTED: 'Rejected',
}
const LEGACY_STATUS_LABEL = 'Historical'

export function requestFormatLabel(format: string | null | undefined): string {
	switch (format) {
		case 'EBOOK':
			return 'Ebook'
		case 'AUDIOBOOK':
			return 'Audiobook'
		default:
			return 'Either'
	}
}

export const REQUEST_STATUS_VARIANTS: Record<
	string,
	'default' | 'secondary' | 'outline' | 'destructive'
> = {
	PENDING: 'outline',
	APPROVED: 'secondary',
	REJECTED: 'destructive',
}

export const REQUEST_STATUS_DESCRIPTIONS: Record<string, string> = {
	PENDING: 'Waiting for an operator to review it.',
	APPROVED: 'Approved. A manager can add a release, which lands in the Editor for review.',
	REJECTED: 'Rejected by an operator.',
}
const LEGACY_STATUS_DESCRIPTION = 'Recorded by an earlier workflow; no further action is taken.'

const APPROVAL_POLICY_LABELS: Record<string, string> = {
	REQUIRED: 'Approval required',
	NONE: 'Auto-approved',
	NOT_REQUIRED: 'Auto-approved',
	AUTO: 'Auto-approved',
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

/** `SOME_NEW_POLICY` → `Some New Policy`: readable words for a value the client has no label for. */
function titleWords(id: string): string {
	return id
		.split(/[-_\s]+/)
		.filter(Boolean)
		.map((word) => word[0].toUpperCase() + word.slice(1).toLowerCase())
		.join(' ')
}

export function statusLabel(status: string | null | undefined): string {
	const normalized = status?.toUpperCase() ?? ''
	if (LEGACY[normalized]) return LEGACY_STATUS_LABEL
	return (
		REQUEST_STATUS_LABELS[normalized] ??
		(normalized ? safeText(normalized.replace(/_/g, ' ')) : 'Unknown status')
	)
}

export function statusDescription(status: string | null | undefined): string {
	const normalized = status?.toUpperCase() ?? ''
	if (LEGACY[normalized]) return LEGACY_STATUS_DESCRIPTION
	return REQUEST_STATUS_DESCRIPTIONS[normalized] ?? 'Status recorded by the server.'
}

/** Only `REQUIRED` is written today; the rest guard against a server that starts auto-approving. */
export function approvalPolicyLabel(policy: string | null | undefined): string {
	const normalized = safeText(policy, 80).toUpperCase()
	if (!normalized) return 'Unknown policy'
	return APPROVAL_POLICY_LABELS[normalized] ?? titleWords(normalized)
}

/**
 * A provider's display name; empty when there is no provider. `sourceProvider`
 * is typed by hand on the create form, so it is sanitised before lookup.
 */
export function providerLabel(id: string | null | undefined): string {
	const clean = safeText(id, 80)
	return clean ? sharedProviderLabel(clean.toLowerCase()) : ''
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

/** What a provider says exists for a work; `null`/missing means it does not say. */
export type FormatAvailability = {
	hasEbook?: boolean | null
	hasAudiobook?: boolean | null
	/** Length of the default audiobook edition. */
	audioSeconds?: number | null
}

/**
 * The format a request starts with: the formats known to exist, both when
 * both are known (or neither is), and `ANY` again when both are known to be
 * missing so a request still names at least one format.
 */
export function defaultRequestFormat(availability: FormatAvailability): RequestFormat {
	// A provider that says “yes” outranks silence, which outranks “no”.
	const ebook = availability.hasEbook === true ? 2 : availability.hasEbook === false ? 0 : 1
	const audiobook = availability.hasAudiobook === true ? 2 : availability.hasAudiobook === false ? 0 : 1
	if (ebook === audiobook) return 'ANY'
	return ebook > audiobook ? 'EBOOK' : 'AUDIOBOOK'
}

/** A request format that can carry a narrator preference. */
export function formatIncludesAudio(format: string | null | undefined): boolean {
	return format === 'AUDIOBOOK' || format === 'ANY'
}

/** `11h 40m` (`40m` under an hour); `null` when the length is unknown or zero. */
export function audioLengthLabel(seconds: number | null | undefined): string | null {
	if (!seconds || seconds <= 0) return null
	const minutes = Math.round(seconds / 60)
	const hours = Math.floor(minutes / 60)
	if (hours === 0) return `${minutes}m`
	return `${hours}h ${minutes % 60}m`
}

/** Identifies a work for `audiobookNarrators`. */
export type NarratorLookup = {
	provider: string
	remoteId: string
	title: string
	authors?: string | null
}

/** The statuses after which a request's narrator preference is settled. */
const NARRATOR_SETTLED_STATUSES: Record<string, true> = { COMPLETED: true, REJECTED: true }

/** The narrator can still be changed on a request of `status` that includes audio. */
export function canEditNarrator(format: string | null | undefined, status: string | null | undefined): boolean {
	return formatIncludesAudio(format) && !NARRATOR_SETTLED_STATUSES[status?.toUpperCase() ?? '']
}
