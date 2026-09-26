import type { LucideIcon } from '@lucide/svelte'
import ActivityIcon from '@lucide/svelte/icons/activity'
import AudioLinesIcon from '@lucide/svelte/icons/audio-lines'
import BookAudioIcon from '@lucide/svelte/icons/book-audio'
import BookMarkedIcon from '@lucide/svelte/icons/book-marked'
import BookOpenTextIcon from '@lucide/svelte/icons/book-open-text'
import CpuIcon from '@lucide/svelte/icons/cpu'
import DownloadIcon from '@lucide/svelte/icons/download'
import GlobeIcon from '@lucide/svelte/icons/globe'
import HighlighterIcon from '@lucide/svelte/icons/highlighter'
import LibraryIcon from '@lucide/svelte/icons/library'
import ListTreeIcon from '@lucide/svelte/icons/list-tree'
import MonitorIcon from '@lucide/svelte/icons/monitor'
import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw'
import RssIcon from '@lucide/svelte/icons/rss'
import ShieldCheckIcon from '@lucide/svelte/icons/shield-check'
import SmartphoneIcon from '@lucide/svelte/icons/smartphone'
import SquareTerminalIcon from '@lucide/svelte/icons/square-terminal'
import TabletIcon from '@lucide/svelte/icons/tablet'

import type {
	CreateDeviceMutation,
	DeviceKind,
	DeviceProtocol,
	DevicesQuery,
	UserPermission,
} from '$lib/graphql/generated/graphql'
import { bytesLabel, countNoun } from '$lib/format'

export type Device = DevicesQuery['devices'][number]
/** A device with its freshly minted secret and endpoints (`createDevice`/`rotateDeviceCredential`). */
export type IssuedCredential = CreateDeviceMutation['createDevice']

/** CrossPoint is its own registered device kind; its keyed sync lane stays KOReader-compatible. */
export type CatalogDeviceKind = DeviceKind | 'CROSSPOINT' | 'COPPICE'

/**
 * The server-side capability snapshot used by the add-client catalog. Keep
 * this adapter deliberately structural: GraphQL codegen owns its generated
 * operation types, while the picker only needs this small, stable contract.
 */
export interface DeviceCapabilityDescriptor {
	kind: string
	protocol: string
	componentKey: string
	compiled: boolean
	enabled: boolean
	available: boolean
	reason?: string | null
}

/**
 * Component keys are the capability boundary, not the device kind label. A
 * single server integration can back several clients (for example Komelia and
 * Mihon share Komga), while pairing profiles such as COPPICE need more than
 * one existing protocol lane.
 */
const COMPONENT_KEYS_FOR_KIND: Record<CatalogDeviceKind, readonly string[]> = {
	KOBO: ['kobo'],
	KOREADER: ['koreader'],
	CROSSPOINT: ['crosspoint'],
	COPPICE: ['liseur-sync', 'api'],
	MIHON: ['komga'],
	KOMELIA: ['komga'],
	LISEUR: ['liseur-sync'],
	OPDS: ['opds'],
	ABS: ['abs'],
	API: ['api'],
	WEB: ['webui'],
	WORKER: ['worker'],
	KAVITA: ['kavita'],
	SOURCE_WORKER: [],
}

export function capabilityForKind(
	kind: CatalogDeviceKind,
	capabilities: readonly DeviceCapabilityDescriptor[] | null | undefined,
): DeviceCapabilityDescriptor | null {
	if (!capabilities) return null
	return (
		capabilities.find((capability) => capability.kind === kind) ??
		capabilities.find((capability) =>
			COMPONENT_KEYS_FOR_KIND[kind]?.includes(capability.componentKey),
		) ??
		null
	)
}

export type ClientCapabilityAvailability = {
	available: boolean
	reason: string | null
}

export function clientCapabilityAvailability(
	kind: CatalogDeviceKind,
	capabilities: readonly DeviceCapabilityDescriptor[] | null | undefined,
): ClientCapabilityAvailability {
	// Browser credentials are still backed by the server's `webui` descriptor,
	// so a disabled Web component must not leave setup enabled accidentally.
	if (!capabilities) {
		return { available: false, reason: 'Server capabilities are still loading.' }
	}

	const keys = COMPONENT_KEYS_FOR_KIND[kind] ?? []
	const matching = capabilities.filter(
		(capability) => capability.kind === kind || keys.includes(capability.componentKey),
	)
	if (!matching.length) {
		return { available: false, reason: 'This integration is not advertised by the server.' }
	}

	const unavailable = matching.find((capability) => !capability.available)
	if (unavailable) {
		return {
			available: false,
			reason: unavailable.reason || 'This integration is disabled on the server.',
		}
	}
	return { available: true, reason: null }
}

export const HIDE_DISABLED_CLIENTS_STORAGE_KEY = 'coppice.devices.hide-disabled-integrations'

export const DEVICE_KIND_LABELS: Record<CatalogDeviceKind, string> = {
	KOBO: 'Kobo',
	KOREADER: 'KOReader',
	CROSSPOINT: 'CrossPoint',
	COPPICE: 'Coppice',
	MIHON: 'Mihon',
	KOMELIA: 'Komelia',
	LISEUR: 'Liseur',
	OPDS: 'OPDS reader',
	ABS: 'Audiobookshelf client',
	API: 'API client',
	WEB: 'Browser',
	WORKER: 'Worker',
	KAVITA: 'Kavita',
	SOURCE_WORKER: 'Source worker',
}

/** The lucide icon that stands for each kind on cards, tiles, and lists. */
export const DEVICE_KIND_ICONS: Record<CatalogDeviceKind, LucideIcon> = {
	KOBO: TabletIcon,
	KOREADER: BookOpenTextIcon,
	CROSSPOINT: TabletIcon,
	COPPICE: TabletIcon,
	MIHON: SmartphoneIcon,
	KOMELIA: MonitorIcon,
	LISEUR: BookMarkedIcon,
	OPDS: RssIcon,
	ABS: BookAudioIcon,
	API: SquareTerminalIcon,
	WEB: GlobeIcon,
	WORKER: CpuIcon,
	KAVITA: MonitorIcon,
	SOURCE_WORKER: CpuIcon,
}

/** The wire protocol a credential was minted for, as the chip on a card reads it. */
export const PROTOCOL_LABELS: Record<DeviceProtocol, string> = {
	KOBO: 'Kobo sync',
	KOREADER: 'KOReader sync',
	KOMGA: 'Komga API',
	OPDS: 'OPDS',
	LISEUR: 'liseur-sync',
	API: 'API key',
}

/** The protocol implied by a device kind, including revoked devices with no credential row. */
export function protocolForKind(kind: CatalogDeviceKind): DeviceProtocol {
	switch (kind) {
		case 'KOBO':
			return 'KOBO'
		case 'KOREADER':
		case 'CROSSPOINT':
			return 'KOREADER'
		case 'COPPICE':
			return 'LISEUR'
		case 'MIHON':
		case 'KOMELIA':
			return 'KOMGA'
		case 'OPDS':
			return 'OPDS'
		case 'LISEUR':
			return 'LISEUR'
		case 'ABS':
		case 'API':
		case 'WEB':
		case 'WORKER':
		case 'SOURCE_WORKER':
		case 'KAVITA':
			return 'API'
	}
}

/**
 * The permissions needed to mint a credential for a kind. These mirror the
 * server's `required_permissions` mapping: the credential can never grant
 * rights that the owning account does not already have.
 */
const KIND_PERMISSIONS: Partial<Record<CatalogDeviceKind, readonly UserPermission[]>> = {
	KOBO: ['ACCESS_API_KEYS', 'ACCESS_KOBO_SYNC', 'DOWNLOAD_FILE'],
	KOREADER: ['ACCESS_API_KEYS', 'ACCESS_KOREADER_SYNC'],
	CROSSPOINT: [],
	COPPICE: [],
	MIHON: ['ACCESS_API_KEYS', 'DOWNLOAD_FILE'],
	KOMELIA: ['ACCESS_API_KEYS', 'DOWNLOAD_FILE'],
	OPDS: ['ACCESS_API_KEYS', 'DOWNLOAD_FILE'],
	ABS: ['ACCESS_API_KEYS', 'DOWNLOAD_FILE'],
	API: ['ACCESS_API_KEYS'],
	WEB: ['ACCESS_API_KEYS'],
	WORKER: ['ACCESS_API_KEYS', 'ACCESS_WORKER', 'DOWNLOAD_FILE'],
	SOURCE_WORKER: ['ACCESS_API_KEYS', 'ACCESS_REMOTE_SOURCE'],
	KAVITA: ['ACCESS_API_KEYS', 'DOWNLOAD_FILE'],
}

export const PERMISSION_LABELS: Partial<Record<UserPermission, string>> = {
	ACCESS_API_KEYS: 'API keys',
	ACCESS_KOBO_SYNC: 'Kobo sync',
	ACCESS_KOREADER_SYNC: 'KOReader sync',
	ACCESS_WORKER: 'worker access',
	ACCESS_REMOTE_SOURCE: 'remote source access',
	DOWNLOAD_FILE: 'download files',
}

export function requiredPermissionsForKind(kind: CatalogDeviceKind): readonly UserPermission[] {
	return KIND_PERMISSIONS[kind] ?? []
}

export function missingPermissionsForKind(
	kind: CatalogDeviceKind,
	user: { isServerOwner: boolean; permissions: readonly UserPermission[] } | null,
): UserPermission[] {
	if (!user || user.isServerOwner) return []
	return requiredPermissionsForKind(kind).filter(
		(permission) => !user.permissions.includes(permission),
	)
}

export type ClientPlatformFilter = 'ios' | 'android' | 'ereaders' | 'desktop' | 'automation'
/** Kept as the public name used by the add-device flow. */
export type ClientFilter = ClientPlatformFilter
export type ClientReadFilter = 'comics' | 'ebooks' | 'audiobooks'
export type ClientConnectionMode = 'credential' | 'pairing'
export type ClientCapabilityStatus = 'full' | 'partial' | 'unsupported' | 'unverified'
export type ClientCapabilityGroup = 'sync' | 'reads'
export type ClientCapabilityId =
	| 'catalog'
	| 'downloads'
	| 'progress'
	| 'shelves'
	| 'annotations'
	| 'sessions'
	| 'transcode'
	| 'alignment'
	| 'challenge'
	| 'narration'
	| 'rich-sync'
	| 'physical-client'
export type ClientMediaFormat = 'comics' | 'ebooks' | 'audiobooks'
export type ClientMaturity = 'stable' | 'beta' | 'preview' | 'unverified'

export interface ClientCapability {
	id: ClientCapabilityId
	label: string
	group: ClientCapabilityGroup
	status: ClientCapabilityStatus
	/** Explains a partial/unsupported/unverified indicator to screen readers and sighted users. */
	detail?: string
	icon: LucideIcon
}

export interface ClientMediaCapability {
	format: ClientMediaFormat
	label: string
	status: ClientCapabilityStatus
	/** Explains a partial/unsupported/unverified indicator to screen readers and sighted users. */
	detail?: string
	icon: LucideIcon
}

export interface ClientGuide {
	label: string
	href: string
}

export interface SetupHint {
	/** Short imperative steps to run on the device once the credential exists. */
	steps: readonly string[]
	/** The guide that walks through those steps, opened in a new tab. */
	docs?: ClientGuide
	/** A safe, origin-only setup archive that can be generated in Home. */
	download?: 'coppice'
}

export type ClientEvidenceLabel = 'Device tested' | 'Contract checked' | 'Preview'

export interface ClientEvidence {
	label: ClientEvidenceLabel
	/** One short accessible explanation of what was and was not exercised. */
	detail: string
}

/** An exact app/profile inside a family card. */
export interface ClientAppVariant {
	id: string
	label: string
	kind: CatalogDeviceKind
	icon?: LucideIcon
	platforms: readonly string[]
	filters: readonly ClientFilter[]
	capabilities: readonly ClientCapability[]
	mediaFormats: readonly ClientMediaCapability[]
	protocol: string
	setup: string
	setupProfile: SetupHint
	guide?: ClientGuide
	keywords: readonly string[]
	connection: ClientConnectionMode
	evidence: ClientEvidence
	maturity: ClientMaturity
	caveat?: string
}

/**
 * Presentation-level client catalog entry. A family card can expose exact app
 * variants; the add flow persists the selected variant's exact kind.
 */
export interface ClientCatalogEntry {
	/** Stable id for selection, analytics-free URLs, and keyed rendering. */
	id: string
	/** One exact DeviceKind or the exact variants represented by this entry. */
	kind: CatalogDeviceKind | readonly CatalogDeviceKind[]
	icon: LucideIcon
	title: string
	description: string
	supportedApps: readonly string[]
	/** Concrete app/device environments rather than broad marketing labels. */
	platforms: readonly string[]
	filters: readonly ClientFilter[]
	capabilities: readonly ClientCapability[]
	mediaFormats: readonly ClientMediaCapability[]
	/** Compact protocol/connection label shown on the card. */
	protocol: string
	/** One-sentence setup summary shown before the detailed profile. */
	setup: string
	setupProfile: SetupHint
	guide?: ClientGuide
	keywords: readonly string[]
	connection: ClientConnectionMode
	evidence: ClientEvidence
	maturity: ClientMaturity
	/** Support boundary shown once the client is chosen (step 2), never on the compact card. */
	caveat?: string
	/** Exact app variants represented by this family card. */
	variants?: readonly ClientAppVariant[]
	defaultVariantId?: string
}

const capability = (
	id: ClientCapabilityId,
	label: string,
	group: ClientCapabilityGroup,
	icon: LucideIcon,
	status: ClientCapabilityStatus = 'full',
	detail?: string,
): ClientCapability => ({ id, label, group, status, detail, icon })

const media = (
	format: ClientMediaFormat,
	label: string,
	icon: LucideIcon,
	status: ClientCapabilityStatus = 'full',
	detail?: string,
): ClientMediaCapability => ({ format, label, status, detail, icon })

const fullMedia = (format: ClientMediaFormat, label: string, icon: LucideIcon) =>
	media(format, label, icon)
const partialMedia = (format: ClientMediaFormat, label: string, icon: LucideIcon, detail: string) =>
	media(format, label, icon, 'partial', detail)
const unsupportedMedia = (
	format: ClientMediaFormat,
	label: string,
	icon: LucideIcon,
	detail: string,
) => media(format, label, icon, 'unsupported', detail)
const unverifiedMedia = (
	format: ClientMediaFormat,
	label: string,
	icon: LucideIcon,
	detail: string,
) => media(format, label, icon, 'unverified', detail)

function unique<T>(values: readonly T[]): T[] {
	return [...new Set(values)]
}

function app(value: ClientAppVariant): ClientAppVariant {
	return value
}

type FamilyInput = {
	id: string
	icon: LucideIcon
	title: string
	description: string
	variants: readonly ClientAppVariant[]
	keywords?: readonly string[]
}

function family(input: FamilyInput): ClientCatalogEntry {
	const defaultVariant = input.variants[0]
	if (!defaultVariant) throw new Error(`Client family ${input.id} has no variants`)
	const kinds = unique(input.variants.map((variant) => variant.kind))
	return {
		id: input.id,
		kind: kinds.length === 1 ? kinds[0] : kinds,
		icon: input.icon,
		title: input.title,
		description: input.description,
		supportedApps: input.variants.map((variant) => variant.label),
		platforms: unique(input.variants.flatMap((variant) => variant.platforms)),
		filters: unique(input.variants.flatMap((variant) => variant.filters)),
		capabilities: defaultVariant.capabilities,
		mediaFormats: defaultVariant.mediaFormats,
		protocol: defaultVariant.protocol,
		setup: defaultVariant.setup,
		setupProfile: defaultVariant.setupProfile,
		guide: defaultVariant.guide,
		keywords: unique([
			input.title,
			...(input.keywords ?? []),
			...input.variants.flatMap((variant) => variant.keywords),
		]),
		connection: defaultVariant.connection,
		evidence: defaultVariant.evidence,
		maturity: defaultVariant.maturity,
		caveat: defaultVariant.caveat,
		variants: input.variants,
		defaultVariantId: defaultVariant.id,
	}
}

const setup = {
	liseur: {
		steps: ['Point Liseur at the server URL and paste the issued liseur-sync token.'],
		docs: { label: 'Liseur integration guide', href: '/docs/developer/liseur-sync-integration' },
	},
	koboStock: {
		steps: [
			'Open the Kobo stock sync settings and point api_endpoint at the server URL above.',
			'Keep the device connected while the stock reader performs its normal sync.',
		],
		docs: { label: 'Kobo integration guide', href: '/docs/guides/integrations/kobo' },
	},
	kobo: {
		steps: [
			'Connect the Kobo over USB and install the NickelCoppice package from KoboRoot.tgz.',
			'Open NickelCoppice on the Kobo and start a five-minute pairing request.',
			'Approve the six-digit code in Home; the plugin stores its scoped credentials on-device.',
		],
		docs: { label: 'Kobo integration guide', href: '/docs/guides/integrations/kobo' },
	},
	koreader: {
		steps: [
			'Copy the coppice.koplugin folder into KOReader/plugins and restart KOReader.',
			'Open Coppice, download or browse once, then choose Pair to display a six-digit code.',
			'Approve the code in Home; the plugin stores the API key and liseur token, then configures built-in KOSync.',
		],
		docs: { label: 'KOReader integration guide', href: '/docs/guides/integrations/koreader' },
		download: 'coppice' as const,
	},
	crosspoint: {
		steps: [
			'On CrossPoint, start File Transfer / Calibre Wireless and choose Coppice pairing.',
			'Approve the six-digit code in Home within five minutes; pairing sends the keyed KOReader-compatible sync URL to the device.',
			'Use the CrossPoint target panel in Home to verify its private IPv4 address before queuing LAN delivery.',
		],
		docs: { label: 'CrossPoint integration guide', href: '/docs/guides/integrations/crosspoint' },
	},
	koreaderSync: {
		steps: [
			'In KOReader open Progress sync, then Custom sync server, and paste the URL above.',
			'Use the issued username and API key as the sync credentials, then pick Binary document matching.',
		],
		docs: { label: 'KOReader sync guide', href: '/docs/guides/integrations/koreader' },
	},
	komga: {
		steps: ['Add a Komga server with the URL above and authenticate with the issued API key.'],
	},
	kavita: {
		steps: [
			'Choose the app’s Kavita /api profile, enter the server URL, and authenticate with the issued API key.',
		],
		docs: { label: 'Platform and client guide', href: '/docs/developer/platforms' },
	},
	opds: {
		steps: [
			'Add a catalog in the reader from the OPDS 1.2 URL, or scan its QR code.',
			'Use the issued API key for OPDS authentication; readers that support OPDS 2.0 can use its feed URL.',
		],
	},
	abs: {
		steps: ['Enter the server URL, then authenticate Audiobookshelf with the issued API key.'],
	},
	api: {
		steps: ['Use the issued API key as a Bearer token against the documented API routes.'],
	},
	browser: {
		steps: ['Open the server URL and sign in with your Coppice account.'],
	},
	worker: {
		steps: [
			'Run stump-worker with the server URL and the issued API key.',
			'Add --ffmpeg or --chrome only for tools installed on that machine.',
		],
		docs: { label: 'Remote worker guide', href: '/docs/developer/workers' },
	},
}

const readMedia = {
	comics: (status: ClientCapabilityStatus = 'full', detail?: string) =>
		media('comics', 'Comics', BookOpenTextIcon, status, detail),
	ebooks: (status: ClientCapabilityStatus = 'full', detail?: string) =>
		media('ebooks', 'eBooks', BookMarkedIcon, status, detail),
	audiobooks: (
		status: ClientCapabilityStatus = 'unverified',
		detail = 'No app-specific audiobook run is recorded.',
	) => media('audiobooks', 'Audiobooks', AudioLinesIcon, status, detail),
}

/** All client choices offered by Home, with Liseur intentionally first. */
export const CLIENT_CATALOG: readonly ClientCatalogEntry[] = [
	family({
		id: 'liseur',
		icon: BookMarkedIcon,
		title: 'Liseur',
		description:
			'A native reader profile for catalog browsing, reading state, and liseur-sync annotations.',
		variants: [
			app({
				id: 'liseur-native',
				label: 'Liseur',
				kind: 'LISEUR',
				platforms: ['Android'],
				filters: ['android'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
					capability('annotations', 'Annotations', 'sync', HighlighterIcon),
					capability('sessions', 'Reading state', 'sync', RefreshCwIcon),
				],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'The native reader profile is documented for books; comic support is not separately verified.',
					),
					fullMedia('ebooks', 'eBooks', BookMarkedIcon),
					unsupportedMedia(
						'audiobooks',
						'Audiobooks',
						AudioLinesIcon,
						'The native Liseur profile does not expose an audiobook player.',
					),
				],
				protocol: 'liseur-sync',
				setup:
					'Pair Liseur with a scoped liseur-sync token; no account password is placed in the client.',
				setupProfile: setup.liseur,
				keywords: ['native reader', 'annotation', 'read state'],
				connection: 'pairing',
				evidence: {
					label: 'Device tested',
					detail: 'Liseur was exercised against the Komga, OPDS 1.2, and liseur-sync profiles.',
				},
				maturity: 'stable',
				caveat:
					'The exact app profile controls whether OPDS, Komga, or native liseur-sync features are shown.',
			}),
		],
		keywords: ['first-party', 'liseur-sync'],
	}),
	family({
		id: 'kobo-stock',
		icon: TabletIcon,
		title: 'Kobo stock sync',
		description:
			'Server-driven catalog, downloads, progress, and shelf-shaped metadata for the stock Kobo reader.',
		variants: [
			app({
				id: 'kobo-stock-reader',
				label: 'Kobo stock reader',
				kind: 'KOBO',
				platforms: ['Kobo eReaders'],
				filters: ['ereaders'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
					capability('shelves', 'Shelves', 'sync', ListTreeIcon),
				],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'No physical Kobo run is claimed for comic delivery.',
					),
					fullMedia('ebooks', 'eBooks', BookMarkedIcon),
					unsupportedMedia(
						'audiobooks',
						'Audiobooks',
						AudioLinesIcon,
						'The stock Kobo sync profile does not deliver audiobooks.',
					),
				],
				protocol: 'Kobo sync',
				setup:
					'Point Kobo stock sync at the server endpoint; a physical Kobo run is intentionally deferred.',
				setupProfile: setup.koboStock,
				keywords: ['stock kobo', 'kepub', 'shelf'],
				connection: 'credential',
				evidence: {
					label: 'Contract checked',
					detail:
						'Kobo initialization, sync, KEPUB, and range contracts are replayed; no physical Kobo run is claimed.',
				},
				maturity: 'preview',
				caveat:
					'Physical Kobo verification is deferred; install and approval instructions are shown separately from contract evidence.',
			}),
		],
	}),
	family({
		id: 'nickelcoppice',
		icon: TabletIcon,
		title: 'NickelCoppice',
		description:
			'The Kobo companion hook for pairing, catalog/download credentials, and liseur-sync read state.',
		variants: [
			app({
				id: 'nickelcoppice',
				label: 'NickelCoppice',
				kind: 'COPPICE',
				platforms: ['Kobo eReaders'],
				filters: ['ereaders'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
					capability('annotations', 'Annotations', 'sync', HighlighterIcon),
					capability('sessions', 'Reading state', 'sync', RefreshCwIcon),
				],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'No physical Kobo run is claimed for comic delivery.',
					),
					fullMedia('ebooks', 'eBooks', BookMarkedIcon),
					unsupportedMedia(
						'audiobooks',
						'Audiobooks',
						AudioLinesIcon,
						'NickelCoppice does not expose an audiobook player.',
					),
				],
				protocol: 'liseur-sync + API key',
				setup:
					'Install KoboRoot.tgz, start pairing on-device, and approve the six-digit code in Home.',
				setupProfile: setup.kobo,
				keywords: ['kobo', 'nickelcoppice', 'koboroot.tgz', 'pairing'],
				connection: 'pairing',
				evidence: {
					label: 'Contract checked',
					detail:
						'The pairing and credential contracts are checked; physical Kobo verification is deferred.',
				},
				maturity: 'preview',
				caveat: 'The archive remains KoboRoot.tgz; do not co-install obsolete NickelStump hooks.',
			}),
		],
	}),
	family({
		id: 'koreader-sync',
		icon: BookOpenTextIcon,
		title: 'KOReader · built-in KOSync',
		description:
			'The built-in KOReader progress-sync adapter using one API key and Binary document matching.',
		variants: [
			app({
				id: 'koreader-kosync',
				label: 'KOReader built-in KOSync',
				kind: 'KOREADER',
				platforms: ['KOReader devices'],
				filters: ['ereaders'],
				capabilities: [capability('progress', 'Progress', 'sync', ActivityIcon)],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'The built-in KOSync lane synchronizes reading position only.',
					),
					fullMedia('ebooks', 'eBooks', BookMarkedIcon),
					unsupportedMedia(
						'audiobooks',
						'Audiobooks',
						AudioLinesIcon,
						'KOSync does not deliver audiobooks.',
					),
				],
				protocol: 'KOReader sync',
				setup:
					'Use the issued API key in KOReader Progress sync; this built-in lane remains a single-key flow.',
				setupProfile: setup.koreaderSync,
				keywords: ['koreader', 'kosync', 'progress'],
				connection: 'credential',
				evidence: {
					label: 'Contract checked',
					detail:
						'The KOSync route and Binary matching contract are checked; no physical KOReader run is claimed.',
				},
				maturity: 'preview',
				caveat:
					'This is intentionally separate from coppice.koplugin: it keeps the existing one-API-key KOSync behavior.',
			}),
		],
	}),
	family({
		id: 'koreader-plugin',
		icon: BookOpenTextIcon,
		title: 'KOReader · coppice.koplugin',
		description:
			'The first-party KOReader companion for catalog, downloads, progress, and annotations.',
		variants: [
			app({
				id: 'coppice-koplugin',
				label: 'coppice.koplugin',
				kind: 'COPPICE',
				platforms: ['KOReader devices', 'Android', 'Desktop'],
				filters: ['ereaders', 'android', 'desktop'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
					capability('annotations', 'Annotations', 'sync', HighlighterIcon),
					capability('sessions', 'Reading state', 'sync', RefreshCwIcon),
				],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'The plugin handles book downloads; a device-specific comic run is not claimed.',
					),
					fullMedia('ebooks', 'eBooks', BookMarkedIcon),
					unsupportedMedia(
						'audiobooks',
						'Audiobooks',
						AudioLinesIcon,
						'The plugin does not expose an audiobook player.',
					),
				],
				protocol: 'liseur-sync + API key',
				setup:
					'Download the origin-only archive, install it, then pair on-device so Home never handles plugin secrets.',
				setupProfile: setup.koreader,
				keywords: ['koreader', 'coppice.koplugin', 'pairing', 'kosync', 'annotations'],
				connection: 'pairing',
				evidence: {
					label: 'Contract checked',
					detail:
						'Plugin pure logic and server routes are checked; KOReader UI/device execution remains unverified.',
				},
				maturity: 'preview',
				caveat:
					'The generated archive contains only the current origin and non-secret defaults. Approval issues both device credentials.',
			}),
		],
	}),
	family({
		id: 'crosspoint',
		icon: TabletIcon,
		title: 'CrossPoint',
		description:
			'CrossPoint eReader pairing with KOReader-compatible progress sync and explicit LAN book delivery.',
		variants: [
			app({
				id: 'crosspoint-reader',
				label: 'CrossPoint',
				kind: 'CROSSPOINT',
				platforms: ['CrossPoint eReaders'],
				filters: ['ereaders'],
				capabilities: [
					capability(
						'progress',
						'KOReader-compatible sync',
						'sync',
						ActivityIcon,
						'full',
						'Uses the existing keyed KOReader-compatible progress lane.',
					),
					capability(
						'rich-sync',
						'Rich sync API',
						'sync',
						RefreshCwIcon,
						'unsupported',
						'The current pinned CrossPoint firmware does not use Coppice rich-sync API URLs.',
					),
					capability(
						'physical-client',
						'Physical client',
						'reads',
						TabletIcon,
						'partial',
						'LAN delivery is available only while File Transfer / Calibre Wireless is active on the device.',
					),
				],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'No physical comic-delivery run is claimed.',
					),
					unverifiedMedia(
						'ebooks',
						'eBooks',
						BookMarkedIcon,
						'Queue delivery can send an EPUB, but no physical CrossPoint run is claimed.',
					),
					unsupportedMedia(
						'audiobooks',
						'Audiobooks',
						AudioLinesIcon,
						'CrossPoint firmware is not an audiobook player.',
					),
				],
				protocol: 'KOReader-compatible sync + LAN transfer',
				setup:
					'Start File Transfer / Calibre Wireless, pair in five minutes, then verify the private IPv4 target in Home.',
				setupProfile: setup.crosspoint,
				keywords: [
					'crosspoint',
					'koreader',
					'kosync',
					'rich sync',
					'lan transfer',
					'calibre wireless',
					'file transfer',
				],
				connection: 'pairing',
				evidence: {
					label: 'Contract checked',
					detail:
						'The keyed KOReader-compatible lane and LAN transfer protocol are checked; physical rich-sync use is not claimed.',
				},
				maturity: 'preview',
				caveat:
					'Current pinned firmware does not use the rich API at a custom URL. Home never collects a Coppice password; LAN delivery requires an explicitly verified private IPv4 target.',
			}),
		],
	}),
	family({
		id: 'komga-apps',
		icon: MonitorIcon,
		title: 'Komga-compatible apps',
		description:
			'Exact Komga app profiles with per-app media support instead of a composite EPUB claim.',
		variants: [
			app({
				id: 'komelia',
				label: 'Komelia',
				kind: 'KOMELIA',
				platforms: ['Android'],
				filters: ['android'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
					capability('shelves', 'Shelves', 'sync', ListTreeIcon),
				],
				mediaFormats: [
					fullMedia('comics', 'Comics', BookOpenTextIcon),
					fullMedia('ebooks', 'eBooks', BookMarkedIcon),
					unsupportedMedia(
						'audiobooks',
						'Audiobooks',
						AudioLinesIcon,
						'Komelia’s Komga profile does not expose an audiobook player.',
					),
				],
				protocol: 'Komga API',
				setup: setup.komga.steps[0],
				setupProfile: setup.komga,
				keywords: ['komga', 'komelia', 'cbz', 'epub'],
				connection: 'credential',
				evidence: {
					label: 'Device tested',
					detail:
						'Komelia 0.19 Android exercised catalog, CBZ/EPUB read and offline download, and progress.',
				},
				maturity: 'stable',
			}),
			app({
				id: 'mihon-komga',
				label: 'Mihon · Komga extension',
				kind: 'MIHON',
				platforms: ['Android'],
				filters: ['android'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
					capability(
						'shelves',
						'Shelves',
						'sync',
						ListTreeIcon,
						'partial',
						'The extension’s tracker/read state is verified; shelf parity is narrower than the full Komga profile.',
					),
				],
				mediaFormats: [
					fullMedia('comics', 'Comics', BookOpenTextIcon),
					unverifiedMedia(
						'ebooks',
						'eBooks',
						BookMarkedIcon,
						'No EPUB reader path is verified for the Mihon Komga extension.',
					),
					unsupportedMedia(
						'audiobooks',
						'Audiobooks',
						AudioLinesIcon,
						'The Mihon Komga extension does not expose an audiobook player.',
					),
				],
				protocol: 'Komga API',
				setup: setup.komga.steps[0],
				setupProfile: setup.komga,
				keywords: ['komga', 'mihon', 'tachiyomi', 'tracker'],
				connection: 'credential',
				evidence: {
					label: 'Device tested',
					detail:
						'Mihon with the Komga extension exercised browse, download, reading, and tracker GET/PUT.',
				},
				maturity: 'stable',
			}),
		],
		keywords: ['komga', 'app profile'],
	}),
	family({
		id: 'kavita-apps',
		icon: MonitorIcon,
		title: 'Kavita-compatible apps',
		description:
			'Choose the exact Kavita app profile; EPUB support is shown only for apps with a Book/* reader path.',
		variants: [
			app({
				id: 'turnleaf',
				label: 'Turnleaf',
				kind: 'KAVITA',
				platforms: ['Android'],
				filters: ['android'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
					capability('shelves', 'Shelves', 'sync', ListTreeIcon),
				],
				mediaFormats: [
					fullMedia('comics', 'Comics', BookOpenTextIcon),
					fullMedia('ebooks', 'eBooks', BookMarkedIcon),
					readMedia.audiobooks(),
				],
				protocol: 'Kavita API',
				setup: setup.kavita.steps[0],
				setupProfile: setup.kavita,
				keywords: ['kavita', 'turnleaf', 'epub'],
				connection: 'credential',
				evidence: {
					label: 'Device tested',
					detail:
						'Turnleaf browse, read, download, and progress flows were exercised against the Kavita profile.',
				},
				maturity: 'stable',
			}),
			app({
				id: 'kamigura',
				label: 'Kamigura',
				kind: 'KAVITA',
				platforms: ['Android'],
				filters: ['android'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
					capability('shelves', 'Shelves', 'sync', ListTreeIcon),
				],
				mediaFormats: [
					fullMedia('comics', 'Comics', BookOpenTextIcon),
					unsupportedMedia(
						'ebooks',
						'eBooks',
						BookMarkedIcon,
						'Kamigura’s image reader lane returns 404 for EPUB.',
					),
					readMedia.audiobooks(),
				],
				protocol: 'Kavita API',
				setup: setup.kavita.steps[0],
				setupProfile: setup.kavita,
				keywords: ['kavita', 'kamigura', 'image reader'],
				connection: 'credential',
				evidence: {
					label: 'Device tested',
					detail:
						'Kamigura browse/read/download flows were exercised; its image lane intentionally does not read EPUB.',
				},
				maturity: 'stable',
				caveat:
					'Kamigura’s image-reader request for EPUB is unsupported; it is not interchangeable with the EPUB-capable profiles.',
			}),
			app({
				id: 'inkita',
				label: 'Inkita',
				kind: 'KAVITA',
				platforms: ['Android'],
				filters: ['android'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
					capability('shelves', 'Shelves', 'sync', ListTreeIcon),
				],
				mediaFormats: [
					fullMedia('comics', 'Comics', BookOpenTextIcon),
					fullMedia('ebooks', 'eBooks', BookMarkedIcon),
					readMedia.audiobooks(),
				],
				protocol: 'Kavita API',
				setup: setup.kavita.steps[0],
				setupProfile: setup.kavita,
				keywords: ['kavita', 'inkita', 'book-page', 'epub'],
				connection: 'credential',
				evidence: {
					label: 'Preview',
					detail:
						'Pinned Inkita source uses Kavita Book/* reader routes; no device run is claimed.',
				},
				maturity: 'unverified',
			}),
			app({
				id: 'kover',
				label: 'Kover',
				kind: 'KAVITA',
				platforms: ['iOS', 'macOS'],
				filters: ['ios', 'desktop'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
					capability('shelves', 'Shelves', 'sync', ListTreeIcon),
				],
				mediaFormats: [
					fullMedia('comics', 'Comics', BookOpenTextIcon),
					fullMedia('ebooks', 'eBooks', BookMarkedIcon),
					readMedia.audiobooks(),
				],
				protocol: 'Kavita API',
				setup: setup.kavita.steps[0],
				setupProfile: setup.kavita,
				keywords: ['kavita', 'kover', 'ios', 'macos', 'epub'],
				connection: 'credential',
				evidence: {
					label: 'Preview',
					detail: 'Pinned Kover source uses Kavita Book/* reader routes; no device run is claimed.',
				},
				maturity: 'unverified',
			}),
			app({
				id: 'kamare',
				label: 'Kamare',
				kind: 'KAVITA',
				platforms: ['KOReader devices'],
				filters: ['ereaders'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
					capability(
						'shelves',
						'Shelves',
						'sync',
						ListTreeIcon,
						'unverified',
						'Pinned source exposes a narrower reader profile; no device run is claimed.',
					),
				],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'Pinned Kamare source uses Kavita image-reader routes; no device run is claimed.',
					),
					unsupportedMedia(
						'ebooks',
						'eBooks',
						BookMarkedIcon,
						'The pinned image-reader profile has no EPUB Book/* route.',
					),
					readMedia.audiobooks(),
				],
				protocol: 'Kavita API',
				setup: setup.kavita.steps[0],
				setupProfile: setup.kavita,
				keywords: ['kavita', 'kamare', 'koreader'],
				connection: 'credential',
				evidence: {
					label: 'Preview',
					detail: 'Pinned Kamare source informed the API profile; no device run is claimed.',
				},
				maturity: 'unverified',
			}),
			app({
				id: 'kareadita',
				label: 'Kareadita · Mihon extension',
				kind: 'KAVITA',
				platforms: ['Android'],
				filters: ['android'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability(
						'progress',
						'Progress',
						'sync',
						ActivityIcon,
						'unverified',
						'Pinned extension source; no device run is claimed.',
					),
					capability(
						'shelves',
						'Shelves',
						'sync',
						ListTreeIcon,
						'unverified',
						'Pinned extension source; no device run is claimed.',
					),
				],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'Pinned extension source uses Kavita image-reader routes; no device run is claimed.',
					),
					unsupportedMedia(
						'ebooks',
						'eBooks',
						BookMarkedIcon,
						'The pinned image-reader profile has no EPUB Book/* route.',
					),
					readMedia.audiobooks(),
				],
				protocol: 'Kavita API',
				setup: setup.kavita.steps[0],
				setupProfile: setup.kavita,
				keywords: ['kavita', 'kareadita', 'mihon', 'tachiyomi'],
				connection: 'credential',
				evidence: {
					label: 'Preview',
					detail:
						'Pinned Kareadita extension source informed the profile; no device run is claimed.',
				},
				maturity: 'unverified',
			}),
			app({
				id: 'mihon-kavita',
				label: 'Mihon · Kavita tracker',
				kind: 'KAVITA',
				platforms: ['Android'],
				filters: ['android'],
				capabilities: [
					capability(
						'catalog',
						'Catalog',
						'reads',
						LibraryIcon,
						'unverified',
						'The tracker is source-derived rather than device-tested.',
					),
					capability(
						'progress',
						'Progress',
						'sync',
						ActivityIcon,
						'unverified',
						'Pinned tracker source uses Kavita Tachiyomi routes; no device run is claimed.',
					),
					capability(
						'shelves',
						'Shelves',
						'sync',
						ListTreeIcon,
						'unsupported',
						'The tracker does not expose a shelf/catalog reader flow.',
					),
				],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'The tracker source is not a reader and has no device run.',
					),
					unsupportedMedia(
						'ebooks',
						'eBooks',
						BookMarkedIcon,
						'A tracker does not expose an EPUB reader.',
					),
					unsupportedMedia(
						'audiobooks',
						'Audiobooks',
						AudioLinesIcon,
						'A tracker does not expose an audiobook player.',
					),
				],
				protocol: 'Kavita tracker',
				setup:
					'Use the app’s Kavita tracker configuration and the issued API key; reader support is not implied.',
				setupProfile: setup.kavita,
				keywords: ['kavita', 'mihon', 'tracker'],
				connection: 'credential',
				evidence: {
					label: 'Preview',
					detail: 'Pinned Mihon tracker source informed the profile; no device run is claimed.',
				},
				maturity: 'unverified',
				caveat:
					'This variant is a tracker integration, not a reader; unsupported media stays explicit.',
			}),
		],
		keywords: ['kavita', 'profile', 'book-page', 'reader image'],
	}),
	family({
		id: 'opds',
		icon: RssIcon,
		title: 'OPDS readers',
		description:
			'Catalog profiles for readers that consume OPDS 1.2 or OPDS 2.0 without claiming universal app parity.',
		variants: [
			app({
				id: 'liseur-opds',
				label: 'Liseur · OPDS',
				kind: 'OPDS',
				platforms: ['Android'],
				filters: ['android'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
				],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'The OPDS profile does not certify comic rendering in every reader.',
					),
					fullMedia('ebooks', 'eBooks', BookMarkedIcon),
					readMedia.audiobooks(),
				],
				protocol: 'OPDS',
				setup: setup.opds.steps[0],
				setupProfile: setup.opds,
				keywords: ['opds', 'liseur', 'pse'],
				connection: 'credential',
				evidence: {
					label: 'Device tested',
					detail: 'Liseur OPDS 1.2 was exercised; a universal OPDS-client claim is not made.',
				},
				maturity: 'stable',
			}),
			app({
				id: 'panels-opds',
				label: 'Panels · OPDS',
				kind: 'OPDS',
				platforms: ['iOS'],
				filters: ['ios'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability(
						'progress',
						'Progress',
						'sync',
						ActivityIcon,
						'unverified',
						'Generic OPDS progress behavior is not device-tested.',
					),
				],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'Panels is listed as an OPDS consumer; no app-specific run is claimed.',
					),
					unverifiedMedia(
						'ebooks',
						'eBooks',
						BookMarkedIcon,
						'No app-specific eBook run is claimed.',
					),
					readMedia.audiobooks(),
				],
				protocol: 'OPDS',
				setup: setup.opds.steps[0],
				setupProfile: setup.opds,
				keywords: ['opds', 'panels', 'ios'],
				connection: 'credential',
				evidence: {
					label: 'Preview',
					detail: 'Panels is an OPDS candidate; no app-specific device run is claimed.',
				},
				maturity: 'unverified',
			}),
			app({
				id: 'chunky-opds',
				label: 'Chunky · OPDS',
				kind: 'OPDS',
				platforms: ['iOS'],
				filters: ['ios'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
				],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'Chunky is listed as an OPDS consumer; no app-specific run is claimed.',
					),
					unverifiedMedia(
						'ebooks',
						'eBooks',
						BookMarkedIcon,
						'No app-specific eBook run is claimed.',
					),
					readMedia.audiobooks(),
				],
				protocol: 'OPDS',
				setup: setup.opds.steps[0],
				setupProfile: setup.opds,
				keywords: ['opds', 'chunky', 'ios'],
				connection: 'credential',
				evidence: {
					label: 'Preview',
					detail: 'Chunky is an OPDS candidate; no app-specific device run is claimed.',
				},
				maturity: 'unverified',
			}),
			app({
				id: 'librera-opds',
				label: 'Librera · OPDS',
				kind: 'OPDS',
				platforms: ['Android'],
				filters: ['android'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
				],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'Librera is listed as an OPDS consumer; no app-specific run is claimed.',
					),
					unverifiedMedia(
						'ebooks',
						'eBooks',
						BookMarkedIcon,
						'No app-specific eBook run is claimed.',
					),
					readMedia.audiobooks(),
				],
				protocol: 'OPDS',
				setup: setup.opds.steps[0],
				setupProfile: setup.opds,
				keywords: ['opds', 'librera', 'android'],
				connection: 'credential',
				evidence: {
					label: 'Preview',
					detail: 'Librera is an OPDS candidate; no app-specific device run is claimed.',
				},
				maturity: 'unverified',
			}),
		],
		keywords: ['opds 1.2', 'opds 2.0', 'catalog'],
	}),
	family({
		id: 'read-aloud-epub',
		icon: BookAudioIcon,
		title: 'Read-aloud EPUB',
		description:
			'Consume accepted, cached EPUB 3 media overlays after a server-side read-aloud job completes.',
		variants: [
			app({
				id: 'read-aloud-opds',
				label: 'Read-aloud EPUB via OPDS',
				kind: 'OPDS',
				platforms: ['Android', 'iOS', 'Desktop'],
				filters: ['android', 'ios', 'desktop'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability(
						'narration',
						'Read aloud',
						'reads',
						BookAudioIcon,
						'partial',
						'Only accepted cached artifacts are exposed; pending jobs are not readable.',
					),
				],
				mediaFormats: [
					unsupportedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'Read-aloud artifacts are EPUB books, not comic archives.',
					),
					fullMedia('ebooks', 'eBooks', BookMarkedIcon),
					unsupportedMedia(
						'audiobooks',
						'Audiobooks',
						AudioLinesIcon,
						'The output is a read-aloud EPUB; standalone audiobook delivery is separate.',
					),
				],
				protocol: 'OPDS',
				setup:
					'Open the accepted cached artifact from OPDS; Home never exposes a pending or unaccepted job.',
				setupProfile: setup.opds,
				keywords: ['read aloud', 'media overlays', 'smil', 'epub'],
				connection: 'credential',
				evidence: {
					label: 'Contract checked',
					detail:
						'Only validated, deterministic cached read-aloud EPUBs are surfaced; physical app playback is not claimed.',
				},
				maturity: 'preview',
				caveat:
					'A folder audiobook is not a ready read-aloud artifact; no GET route starts alignment.',
			}),
		],
	}),
	family({
		id: 'audiobookshelf',
		icon: BookAudioIcon,
		title: 'Audiobookshelf clients',
		description:
			'Audiobook catalog, playback, and progress through the Audiobookshelf-compatible API profile.',
		variants: [
			app({
				id: 'audiobookshelf-client',
				label: 'Audiobookshelf client',
				kind: 'ABS',
				platforms: ['Android', 'iOS'],
				filters: ['android', 'ios'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
					capability('sessions', 'Sessions', 'sync', RefreshCwIcon),
				],
				mediaFormats: [
					unsupportedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'Audiobookshelf is an audiobook client.',
					),
					fullMedia('ebooks', 'eBooks', BookMarkedIcon),
					fullMedia('audiobooks', 'Audiobooks', AudioLinesIcon),
				],
				protocol: 'Audiobookshelf API',
				setup: setup.abs.steps[0],
				setupProfile: setup.abs,
				keywords: ['audiobookshelf', 'audio', 'playback'],
				connection: 'credential',
				evidence: {
					label: 'Contract checked',
					detail: 'The Audiobookshelf route profile and progress/session contracts are checked.',
				},
				maturity: 'preview',
			}),
		],
	}),
	family({
		id: 'browser',
		icon: GlobeIcon,
		title: 'Browser',
		description: 'The Coppice web application for account management and reading-state overview.',
		variants: [
			app({
				id: 'browser-app',
				label: 'Coppice web app',
				kind: 'WEB',
				platforms: ['Desktop', 'Android', 'iOS'],
				filters: ['desktop', 'android', 'ios'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
					capability('annotations', 'Annotations', 'sync', HighlighterIcon),
					capability('sessions', 'Sessions', 'sync', RefreshCwIcon),
				],
				mediaFormats: [
					fullMedia('comics', 'Comics', BookOpenTextIcon),
					fullMedia('ebooks', 'eBooks', BookMarkedIcon),
					unverifiedMedia(
						'audiobooks',
						'Audiobooks',
						AudioLinesIcon,
						'Audiobook playback depends on the web media surface in use.',
					),
				],
				protocol: 'Browser session',
				setup: setup.browser.steps[0],
				setupProfile: setup.browser,
				keywords: ['browser', 'web', 'home'],
				connection: 'credential',
				evidence: {
					label: 'Contract checked',
					detail: 'Home’s authenticated web surface is the first-party browser client.',
				},
				maturity: 'stable',
			}),
		],
	}),
	family({
		id: 'api',
		icon: SquareTerminalIcon,
		title: 'API client',
		description:
			'A scoped API key for scripts or integrations that do not fit a named reader profile.',
		variants: [
			app({
				id: 'api-client',
				label: 'API client',
				kind: 'API',
				platforms: ['Automation'],
				filters: ['automation'],
				capabilities: [
					capability('catalog', 'Catalog', 'reads', LibraryIcon),
					capability('downloads', 'Downloads', 'reads', DownloadIcon),
					capability('progress', 'Progress', 'sync', ActivityIcon),
				],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'The API can expose media, but client rendering is integration-specific.',
					),
					unverifiedMedia(
						'ebooks',
						'eBooks',
						BookMarkedIcon,
						'The API can expose media, but client rendering is integration-specific.',
					),
					unverifiedMedia(
						'audiobooks',
						'Audiobooks',
						AudioLinesIcon,
						'The API can expose media, but client rendering is integration-specific.',
					),
				],
				protocol: 'API key',
				setup: setup.api.steps[0],
				setupProfile: setup.api,
				keywords: ['api', 'script', 'integration'],
				connection: 'credential',
				evidence: {
					label: 'Contract checked',
					detail: 'The API permission and bearer-token contracts are checked.',
				},
				maturity: 'stable',
			}),
		],
	}),
	family({
		id: 'worker',
		icon: CpuIcon,
		title: 'Remote worker',
		description: 'A scoped worker credential for explicit media preparation and read-aloud jobs.',
		variants: [
			app({
				id: 'remote-worker',
				label: 'stump-worker',
				kind: 'WORKER',
				platforms: ['Desktop', 'Automation'],
				filters: ['desktop', 'automation'],
				capabilities: [
					capability('transcode', 'Transcode', 'reads', DownloadIcon),
					capability('alignment', 'Alignment', 'reads', ActivityIcon),
					capability('challenge', 'Challenge', 'sync', ShieldCheckIcon),
				],
				mediaFormats: [
					unverifiedMedia(
						'comics',
						'Comics',
						BookOpenTextIcon,
						'Worker media output depends on the explicit job profile.',
					),
					unverifiedMedia(
						'ebooks',
						'eBooks',
						BookMarkedIcon,
						'Worker media output depends on the explicit job profile.',
					),
					unverifiedMedia(
						'audiobooks',
						'Audiobooks',
						AudioLinesIcon,
						'Worker media output depends on the explicit job profile.',
					),
				],
				protocol: 'Worker API',
				setup: setup.worker.steps[0],
				setupProfile: setup.worker,
				keywords: ['worker', 'alignment', 'transcode', 'read aloud'],
				connection: 'credential',
				evidence: {
					label: 'Contract checked',
					detail:
						'Worker capability and challenge contracts are checked; Storyteller remains optional and worker-local.',
				},
				maturity: 'preview',
			}),
		],
	}),
]

export function deviceKindsForClient(entry: ClientCatalogEntry): readonly CatalogDeviceKind[] {
	return typeof entry.kind === 'string' ? [entry.kind] : entry.kind
}

export function clientVariants(entry: ClientCatalogEntry): readonly ClientAppVariant[] {
	if (entry.variants?.length) return entry.variants
	const kind = deviceKindsForClient(entry)[0]
	if (!kind) return []
	return [
		{
			id: entry.id,
			label: entry.supportedApps[0] ?? entry.title,
			kind,
			icon: entry.icon,
			platforms: entry.platforms,
			filters: entry.filters,
			capabilities: entry.capabilities,
			mediaFormats: entry.mediaFormats,
			protocol: entry.protocol,
			setup: entry.setup,
			setupProfile: entry.setupProfile,
			guide: entry.guide,
			keywords: entry.keywords,
			connection: entry.connection,
			evidence: entry.evidence,
			maturity: entry.maturity,
			caveat: entry.caveat,
		},
	]
}

export function defaultClientVariant(entry: ClientCatalogEntry): ClientAppVariant | null {
	const variants = clientVariants(entry)
	return variants.find((variant) => variant.id === entry.defaultVariantId) ?? variants[0] ?? null
}

export function clientVariantForId(
	entry: ClientCatalogEntry,
	id: string | null | undefined,
): ClientAppVariant | null {
	if (!id) return defaultClientVariant(entry)
	return clientVariants(entry).find((variant) => variant.id === id) ?? defaultClientVariant(entry)
}

/** Merge an exact variant into a profile consumed by AddClientDialog/CredentialReveal. */
export function clientProfileForVariant(
	entry: ClientCatalogEntry,
	variant: ClientAppVariant,
): ClientCatalogEntry {
	return {
		...entry,
		kind: variant.kind,
		title: variant.label,
		supportedApps: [variant.label],
		platforms: variant.platforms,
		filters: variant.filters,
		capabilities: variant.capabilities,
		mediaFormats: variant.mediaFormats,
		protocol: variant.protocol,
		setup: variant.setup,
		setupProfile: variant.setupProfile,
		guide: variant.guide,
		keywords: variant.keywords,
		connection: variant.connection,
		evidence: variant.evidence,
		maturity: variant.maturity,
		caveat: variant.caveat,
	}
}

/** The catalog facts a paired device can show; see `clientProfileForKind`. */
export interface ClientKindProfile {
	evidence: ClientEvidence | null
	maturity: ClientMaturity | null
	capabilities: readonly ClientCapability[]
	mediaFormats: readonly ClientMediaCapability[]
}

/**
 * What a paired device of `kind` is known to do. A device row stores only its
 * kind, and NickelCoppice/coppice.koplugin, the OPDS readers, and the Kavita
 * apps share one, so such a device keeps only the evidence, readiness, and
 * capability statuses every app of that kind agrees on; an explanation that
 * differs between those apps is dropped rather than attributed to the device.
 */
export function clientProfileForKind(kind: CatalogDeviceKind): ClientKindProfile {
	const variants = CLIENT_CATALOG.flatMap((entry) =>
		clientVariants(entry).filter((variant) => variant.kind === kind),
	)
	const [first, ...others] = variants
	if (!first) return { evidence: null, maturity: null, capabilities: [], mediaFormats: [] }
	const agreed = <T extends { status: ClientCapabilityStatus; detail?: string }>(
		item: T,
		peers: (variant: ClientAppVariant) => T | undefined,
	): T | null => {
		const matches = others.map(peers)
		if (!matches.every((match) => match?.status === item.status)) return null
		return matches.every((match) => match?.detail === item.detail)
			? item
			: { ...item, detail: undefined }
	}
	return {
		evidence: others.every(
			(variant) =>
				variant.evidence.label === first.evidence.label &&
				variant.evidence.detail === first.evidence.detail,
		)
			? first.evidence
			: null,
		maturity: others.every((variant) => variant.maturity === first.maturity)
			? first.maturity
			: null,
		capabilities: first.capabilities.flatMap((capability) => {
			const shared = agreed(capability, (variant) =>
				variant.capabilities.find((item) => item.id === capability.id),
			)
			return shared ? [shared] : []
		}),
		mediaFormats: first.mediaFormats.flatMap((format) => {
			const shared = agreed(format, (variant) =>
				variant.mediaFormats.find((item) => item.format === format.format),
			)
			return shared ? [shared] : []
		}),
	}
}

export function missingPermissionsForClient(
	entry: ClientCatalogEntry,
	user: { isServerOwner: boolean; permissions: readonly UserPermission[] } | null,
	kind = deviceKindsForClient(entry)[0],
): UserPermission[] {
	// Pairing creates its own scoped credential after the user approves the
	// request, so it must not be blocked by the credential-minting permissions.
	if (entry.connection === 'pairing' || !kind) return []
	return missingPermissionsForKind(kind, user)
}

/** What to do on the device after the credential is issued, for the kinds that need more than a URL. */
export const SETUP_HINTS: Partial<Record<CatalogDeviceKind, SetupHint>> = {
	KOBO: setup.koboStock,
	KOREADER: setup.koreaderSync,
	COPPICE: setup.koreader,
	MIHON: setup.komga,
	KOMELIA: setup.komga,
	OPDS: setup.opds,
	ABS: setup.abs,
	LISEUR: setup.liseur,
	KAVITA: setup.kavita,
	WORKER: setup.worker,
}

/**
 * The name the server picks for a new device when the user sends none
 * (`DeviceService::create_device`: `"<Username>'s <Kind>"`, numbered on
 * collision). The add flow prefills it so the field is never blank.
 */
export function defaultDeviceName(username: string, kind: CatalogDeviceKind): string {
	return `${username}'s ${DEVICE_KIND_LABELS[kind]}`
}

/** What each stored `lastSyncSummary.protocol` is called when nothing better can be said. */
const SYNC_PROTOCOL_LABELS: Record<string, string> = {
	kobo: 'Kobo sync',
	koreader: 'KOReader sync',
	komga: 'Komga sync',
	kavita: 'Kavita sync',
	abs: 'Audiobookshelf sync',
	opds: 'OPDS sync',
	liseur: 'Liseur sync',
	'kindle-email': 'Kindle delivery',
}

function numberField(record: Record<string, unknown>, key: string): number | null {
	const value = record[key]
	return typeof value === 'number' && Number.isFinite(value) ? value : null
}

/** `43%` from a 0–1 fraction, clamped so a stray 1.2 never reads as 120%. */
function percentLabel(fraction: number): string {
	return `${Math.round(Math.min(Math.max(fraction, 0), 1) * 100)}%`
}

/** `1:02:33` / `2:05` from a millisecond position. */
function positionLabel(ms: number): string {
	const total = Math.max(0, Math.floor(ms / 1000))
	const hours = Math.floor(total / 3600)
	const minutes = Math.floor((total % 3600) / 60)
	const seconds = total % 60
	const pad = (value: number) => String(value).padStart(2, '0')
	return hours ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${minutes}:${pad(seconds)}`
}

/**
 * One human sentence for a device's `lastSyncSummary`, per protocol (shapes in
 * `docs/content/docs/developer/devices.mdx` §Sightings):
 *
 * | protocol / profile | sentence |
 * | --- | --- |
 * | kobo | `Synced 12 items` (`, 3 new` when new entitlements were sent) |
 * | koreader | `Finished` / `Progress 43%` (`document` is the book hash, not a title) |
 * | komga, kavita | `Finished` / `Page 12` / `Progress 43%` (Readium progression) |
 * | opds | `Progress 43%` |
 * | abs | `Position 1:02:33` |
 * | liseur | `7 ops applied` (`, 1 conflict` when any) |
 * | kindle-email | `Sent EPUB (412 KB)` |
 *
 * Anything else falls back to the protocol's name, and a summary with no
 * usable shape to `null` so the card shows only the timestamp.
 */
export function syncSentence(summary: unknown): string | null {
	if (summary === null || summary === undefined) return null
	if (typeof summary === 'string') return summary || null
	if (typeof summary !== 'object') return null
	const record = summary as Record<string, unknown>
	const protocol = typeof record.protocol === 'string' ? record.protocol : null
	const profile = typeof record.profile === 'string' ? record.profile : null
	const fallback = protocol ? (SYNC_PROTOCOL_LABELS[protocol] ?? protocol) : null

	switch (profile ?? protocol) {
		case 'kobo': {
			const items = numberField(record, 'items')
			if (items === null) return fallback
			const fresh = numberField(record, 'new_entitlements')
			return `Synced ${countNoun(items, 'item')}${fresh ? `, ${fresh} new` : ''}`
		}
		case 'koreader': {
			const fraction = numberField(record, 'percentage')
			if (fraction === null) return fallback
			return fraction >= 1 ? 'Finished' : `Progress ${percentLabel(fraction)}`
		}
		case 'komga':
		case 'kavita': {
			if (record.completed === true) return 'Finished'
			const page = numberField(record, 'page')
			if (page !== null) return `Page ${page}`
			const fraction = numberField(record, 'progression')
			return fraction === null ? fallback : `Progress ${percentLabel(fraction)}`
		}
		case 'opds': {
			const fraction = numberField(record, 'progression')
			return fraction === null ? fallback : `Progress ${percentLabel(fraction)}`
		}
		case 'abs': {
			const position = numberField(record, 'position_ms')
			return position === null ? fallback : `Position ${positionLabel(position)}`
		}
		case 'liseur': {
			const applied = numberField(record, 'applied') ?? numberField(record, 'ops')
			if (applied === null) return fallback
			const conflicts = numberField(record, 'conflict')
			return `${countNoun(applied, 'op')} applied${conflicts ? `, ${countNoun(conflicts, 'conflict')}` : ''}`
		}
		case 'kindle-email': {
			const format = typeof record.format === 'string' ? record.format.toUpperCase() : null
			const bytes = numberField(record, 'bytes')
			if (!format) return fallback
			return bytes === null ? `Sent ${format}` : `Sent ${format} (${bytesLabel(bytes)})`
		}
		default:
			return fallback
	}
}

/**
 * Comic transform presets accepted by the server
 * (`stump_media::transform::TransformProfile::preset`). Stored on the device
 * as `{"preset": "<name>"}`; the server also accepts a bare string. `short`
 * is the chip on the card, `label` the full description in the picker.
 */
export const TRANSFORM_PRESETS: { name: string; short: string; label: string }[] = [
	{ name: 'clara', short: 'Clara', label: 'Kobo Clara (1072×1448, grayscale)' },
	{ name: 'libra', short: 'Libra', label: 'Kobo Libra (1264×1680, grayscale)' },
	{ name: 'sage', short: 'Sage', label: 'Kobo Sage (1440×1920, grayscale)' },
	{ name: 'elipsa', short: 'Elipsa', label: 'Kobo Elipsa (1404×1872, grayscale)' },
	{ name: 'nia', short: 'Nia', label: 'Kobo Nia (758×1024, grayscale)' },
	{ name: 'clara-colour', short: 'Clara Colour', label: 'Kobo Clara Colour (1072×1448, colour)' },
	{ name: 'libra-colour', short: 'Libra Colour', label: 'Kobo Libra Colour (1264×1680, colour)' },
	{ name: 'sage-colour', short: 'Sage Colour', label: 'Kobo Sage Colour (1440×1920, colour)' },
	{ name: 'koreader', short: 'KOReader', label: 'KOReader (≤1920×2560, colour WebP CBZ)' },
	{ name: 'phone', short: 'Phone', label: 'Phone (no resize, colour WebP, split tall pages)' },
	{
		name: 'phone-opus',
		short: 'Phone + Opus',
		label: 'Phone + Opus audio (as Phone, audiobooks transcoded to Opus 64k)',
	},
]

/** Kinds whose comics the server transforms per device. */
export const TRANSFORMABLE_KINDS: Partial<Record<DeviceKind, true>> = { KOBO: true, KOREADER: true }

/** The value shown in the preset selector for a device with no preset. */
export const NO_PRESET = 'server-default'

/**
 * The preset name stored in a device's `transformProfile`, or `NO_PRESET`
 * when the device has none (or a hand-written full profile).
 */
export function presetOf(profile: unknown): string {
	if (typeof profile === 'string') return profile
	if (
		profile &&
		typeof profile === 'object' &&
		'preset' in profile &&
		typeof profile.preset === 'string'
	) {
		return profile.preset
	}
	return NO_PRESET
}

/**
 * Opus delivery bitrate of each preset that asks for one
 * (`stump_media::transform::TransformProfile::preset`). Every other preset
 * passes audiobook bytes through as stored, so absence is the default rather
 * than a missing entry.
 */
const PRESET_AUDIO_BITRATE: Record<string, string> = { 'phone-opus': '64k' }

/** How a device's audiobook bytes are delivered, for the device card. */
export const PASSTHROUGH_AUDIO = 'Stored file (no transcode)'

/**
 * The audio delivery a device's `transformProfile` asks for, read the way the
 * server reads it: a preset name (bare or `{"preset": …}`) resolves through
 * the preset table, and a hand-written full profile carries its own
 * `audio.output`.
 */
export function audioDeliveryOf(profile: unknown): string {
	const preset = presetOf(profile)
	if (preset !== NO_PRESET) {
		const bitrate = PRESET_AUDIO_BITRATE[preset]
		return bitrate ? `Opus ${bitrate}` : PASSTHROUGH_AUDIO
	}
	if (!profile || typeof profile !== 'object' || !('audio' in profile)) {
		return PASSTHROUGH_AUDIO
	}
	const audio = profile.audio
	if (!audio || typeof audio !== 'object' || !('output' in audio)) {
		return PASSTHROUGH_AUDIO
	}
	const output = audio.output
	if (!output || typeof output !== 'object' || !('type' in output)) {
		return PASSTHROUGH_AUDIO
	}
	if (output.type !== 'opus') return PASSTHROUGH_AUDIO
	if ('bitrate' in output && typeof output.bitrate === 'string') {
		return `Opus ${output.bitrate}`
	}
	return 'Opus'
}

/**
 * How the device's `libraryScope` reads at a glance. `null` is inherit: the
 * device sees exactly what its user sees. A list is intersected with that
 * visibility server-side, so ids naming a library the user can no longer see
 * are counted but never resolvable — hence the count is of the stored scope,
 * not of what happens to be visible right now.
 */
export function libraryScopeSummary(scope: readonly string[] | null | undefined): string {
	if (!scope) return 'All libraries'
	if (scope.length === 0) return 'No libraries'
	return countNoun(scope.length, 'library', 'libraries')
}
