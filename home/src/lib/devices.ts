import type {
	CreateDeviceMutation,
	DeviceKind,
	DevicesQuery
} from '$lib/graphql/generated/graphql';

export type Device = DevicesQuery['devices'][number];
/** A device with its freshly minted secret and endpoints (`createDevice`/`rotateDeviceCredential`). */
export type IssuedCredential = CreateDeviceMutation['createDevice'];

export const DEVICE_KIND_LABELS: Record<DeviceKind, string> = {
	KOBO: 'Kobo',
	KOREADER: 'KOReader',
	MIHON: 'Mihon',
	KOMELIA: 'Komelia',
	LISEUR: 'Liseur',
	OPDS: 'OPDS reader',
	API: 'API client',
	WEB: 'Browser'
};

/** The kinds a user can register from the Home app, in menu order. */
export const ADDABLE_KINDS: { kind: DeviceKind; description: string }[] = [
	{ kind: 'KOBO', description: 'A Kobo eReader using the native Kobo sync protocol' },
	{ kind: 'KOREADER', description: 'A KOReader install using progress sync' },
	{ kind: 'LISEUR', description: 'Liseur using the native liseur-sync protocol' },
	{ kind: 'MIHON', description: 'Mihon (Tachiyomi) through the Komga-compatible API' },
	{ kind: 'KOMELIA', description: 'Komelia through the Komga-compatible API' },
	{ kind: 'OPDS', description: 'Any OPDS reader (Panels, Chunky, Librera, …)' },
	{ kind: 'API', description: 'A script or integration using the native API' }
];

/**
 * Comic transform presets accepted by the server
 * (`stump_media::transform::TransformProfile::preset`). Stored on the device
 * as `{"preset": "<name>"}`; the server also accepts a bare string.
 */
export const TRANSFORM_PRESETS: { name: string; label: string }[] = [
	{ name: 'clara', label: 'Kobo Clara (1072×1448, grayscale)' },
	{ name: 'libra', label: 'Kobo Libra (1264×1680, grayscale)' },
	{ name: 'sage', label: 'Kobo Sage (1440×1920, grayscale)' },
	{ name: 'elipsa', label: 'Kobo Elipsa (1404×1872, grayscale)' },
	{ name: 'nia', label: 'Kobo Nia (758×1024, grayscale)' },
	{ name: 'clara-colour', label: 'Kobo Clara Colour (1072×1448, colour)' },
	{ name: 'libra-colour', label: 'Kobo Libra Colour (1264×1680, colour)' },
	{ name: 'sage-colour', label: 'Kobo Sage Colour (1440×1920, colour)' },
	{ name: 'koreader', label: 'KOReader (≤1920×2560, colour WebP CBZ)' },
	{ name: 'phone', label: 'Phone (no resize, colour WebP, split tall pages)' }
];

/** Kinds whose comics the server transforms per device. */
export const TRANSFORMABLE_KINDS: Partial<Record<DeviceKind, true>> = { KOBO: true, KOREADER: true };

/** The value shown in the preset selector for a device with no preset. */
export const NO_PRESET = 'server-default';

/**
 * The preset name stored in a device's `transformProfile`, or `NO_PRESET`
 * when the device has none (or a hand-written full profile).
 */
export function presetOf(profile: unknown): string {
	if (typeof profile === 'string') return profile;
	if (profile && typeof profile === 'object' && 'preset' in profile && typeof profile.preset === 'string') {
		return profile.preset;
	}
	return NO_PRESET;
}
