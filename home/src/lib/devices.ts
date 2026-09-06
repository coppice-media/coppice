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
	ABS: 'Audiobookshelf client',
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
	{
		kind: 'ABS',
		description: 'Lissen or another Audiobookshelf client, for audiobooks'
	},
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

/** The value shown in the library selector for a device with no scope. */
export const INHERIT_LIBRARIES = 'inherit';

/**
 * How the device's `libraryScope` reads at a glance. `null` is inherit: the
 * device sees exactly what its user sees. A list is intersected with that
 * visibility server-side, so ids naming a library the user can no longer see
 * are counted but never resolvable — hence the count is of the stored scope,
 * not of what happens to be visible right now.
 */
export function libraryScopeSummary(
	scope: readonly string[] | null | undefined,
	total: number
): string {
	if (!scope) return 'All libraries';
	if (scope.length === 0) return 'No libraries';
	return `${scope.length} of ${total} ${total === 1 ? 'library' : 'libraries'}`;
}

/**
 * What a change in the library selector means for the device's stored scope:
 * `null` to clear the restriction, a list to narrow it, or `undefined` when
 * the selection changed nothing.
 *
 * `INHERIT_LIBRARIES` and a concrete selection are mutually exclusive, and a
 * multi-select reports only the new set — so `current` is what says which
 * entry the user just toggled. Picking "All (inherit)" while restricted
 * clears the scope; picking a library while inheriting starts one.
 * Un-toggling "All (inherit)" on its own means nothing, because a device that
 * sees no library at all should take deselecting them one by one rather than
 * one stray click.
 */
export function nextLibraryScope(
	values: readonly string[],
	current: readonly string[] | null | undefined
): string[] | null | undefined {
	const restricted = current !== null && current !== undefined;
	const picked = values.filter((value) => value !== INHERIT_LIBRARIES);

	if (restricted && values.includes(INHERIT_LIBRARIES)) return null;
	if (!restricted && picked.length === 0) return undefined;
	// Select values are unique, so equal length plus containment is equality.
	const unchanged =
		!!current && picked.length === current.length && picked.every((id) => current.includes(id));
	return unchanged ? undefined : picked;
}
