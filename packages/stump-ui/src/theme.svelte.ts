/**
 * The appearance both companion apps share.
 *
 * Coppice is dark first: with nothing stored the apps render dark, and the
 * two knobs a visitor has are the *mode* (dark, light, or follow the OS) and
 * a *preset*. Most presets tint the neutral scale and pick the accent; the
 * named Coppice and Midnight presets also provide a complete semantic
 * surface treatment (`src/styles/theme.css`). Both choices are kept in
 * `localStorage` under one origin-wide key each, so choosing a theme in the
 * Home app also themes the ingest editor: `/app` and `/editor` are served
 * from the same origin, and an open tab of the other app follows through the
 * `storage` event.
 *
 * `initTheme()` runs once per app, from the root layout. Before any of this
 * loads, the inline `themeBootScript` in `app.html` has already put `.dark`
 * and `data-theme` on `<html>`, so there is no flash of the wrong theme.
 */

export const THEME_MODES = ['dark', 'light', 'system'] as const
export type ThemeMode = (typeof THEME_MODES)[number]

/** What the mode resolves to once `system` has asked the OS. */
export type ResolvedTheme = 'dark' | 'light'

export type ThemePresetKind = 'neutral' | 'accent' | 'special'

export type ThemePresetInfo = {
	id: ThemePreset
	label: string
	kind: ThemePresetKind
	description: string
	/** The colour the switcher paints its swatch with. */
	swatch: string
}

export type ThemePreset =
	| 'neutral'
	| 'zinc'
	| 'slate'
	| 'stone'
	| 'gray'
	| 'blue'
	| 'green'
	| 'orange'
	| 'rose'
	| 'violet'
	| 'amber'
	| 'teal'
	| 'cyan'
	| 'indigo'
	| 'coppice'
	| 'midnight'

/**
 * Every preset, in switcher order: neutral scales, accents, then the two
 * named Coppice palettes. Keeping the kind on the metadata lets the picker
 * distinguish the named palettes without turning every swatch into a card.
 */
export const THEME_PRESETS: readonly ThemePresetInfo[] = [
	{
		id: 'neutral',
		label: 'Neutral',
		kind: 'neutral',
		description: 'Balanced grayscale',
		swatch: 'oklch(0.556 0 0)',
	},
	{
		id: 'zinc',
		label: 'Zinc',
		kind: 'neutral',
		description: 'Cool, soft gray',
		swatch: 'oklch(0.552 0.016 285.938)',
	},
	{
		id: 'slate',
		label: 'Slate',
		kind: 'neutral',
		description: 'Blue-gray neutral',
		swatch: 'oklch(0.554 0.046 257.417)',
	},
	{
		id: 'stone',
		label: 'Stone',
		kind: 'neutral',
		description: 'Warm, earthy neutral',
		swatch: 'oklch(0.553 0.013 58.071)',
	},
	{
		id: 'gray',
		label: 'Gray',
		kind: 'neutral',
		description: 'True gray',
		swatch: 'oklch(0.551 0.027 264.364)',
	},
	{
		id: 'blue',
		label: 'Blue',
		kind: 'accent',
		description: 'Clear blue accent',
		swatch: 'oklch(0.546 0.215 262.9)',
	},
	{
		id: 'green',
		label: 'Green',
		kind: 'accent',
		description: 'Natural green accent',
		swatch: 'oklch(0.545 0.155 162.5)',
	},
	{
		id: 'orange',
		label: 'Orange',
		kind: 'accent',
		description: 'Warm orange accent',
		swatch: 'oklch(0.605 0.192 41.1)',
	},
	{
		id: 'rose',
		label: 'Rose',
		kind: 'accent',
		description: 'Soft rose accent',
		swatch: 'oklch(0.565 0.211 16.5)',
	},
	{
		id: 'violet',
		label: 'Violet',
		kind: 'accent',
		description: 'Vivid violet accent',
		swatch: 'oklch(0.56 0.222 293)',
	},
	{
		id: 'amber',
		label: 'Amber',
		kind: 'accent',
		description: 'Golden amber accent',
		swatch: 'oklch(0.666 0.179 58.318)',
	},
	{
		id: 'teal',
		label: 'Teal',
		kind: 'accent',
		description: 'Calm teal accent',
		swatch: 'oklch(0.6 0.118 184.704)',
	},
	{
		id: 'cyan',
		label: 'Cyan',
		kind: 'accent',
		description: 'Bright cyan accent',
		swatch: 'oklch(0.6 0.13 210)',
	},
	{
		id: 'indigo',
		label: 'Indigo',
		kind: 'accent',
		description: 'Deep indigo accent',
		swatch: 'oklch(0.511 0.262 276.966)',
	},
	{
		id: 'coppice',
		label: 'Coppice',
		kind: 'special',
		description: 'Warm woodland light and dark',
		swatch: 'linear-gradient(135deg, oklch(0.62 0.13 75), oklch(0.42 0.12 165))',
	},
	{
		id: 'midnight',
		label: 'Midnight',
		kind: 'special',
		description: 'Deep blue night palette',
		swatch: 'linear-gradient(135deg, oklch(0.35 0.14 255), oklch(0.2 0.08 285))',
	},
]

export const THEME_MODE_KEY = 'coppice.theme.mode'
export const THEME_PRESET_KEY = 'coppice.theme.preset'

export const DEFAULT_THEME_MODE: ThemeMode = 'dark'
export const DEFAULT_THEME_PRESET: ThemePreset = 'neutral'

const DARK_QUERY = '(prefers-color-scheme: dark)'

function isThemeMode(value: unknown): value is ThemeMode {
	return typeof value === 'string' && (THEME_MODES as readonly string[]).includes(value)
}

function isThemePreset(value: unknown): value is ThemePreset {
	return typeof value === 'string' && THEME_PRESETS.some((preset) => preset.id === value)
}

function read<T>(key: string, guard: (value: unknown) => value is T, fallback: T): T {
	try {
		const stored = localStorage.getItem(key)
		return guard(stored) ? stored : fallback
	} catch {
		// Private mode, disabled storage: the default is a fine answer.
		return fallback
	}
}

function store(key: string, value: string): void {
	try {
		localStorage.setItem(key, value)
	} catch {
		// Nothing to recover: the choice simply does not survive a reload.
	}
}

class ThemeState {
	#mode = $state<ThemeMode>(DEFAULT_THEME_MODE)
	#preset = $state<ThemePreset>(DEFAULT_THEME_PRESET)
	#prefersDark = $state(false)

	constructor() {
		if (typeof window === 'undefined') return
		this.#mode = read(THEME_MODE_KEY, isThemeMode, DEFAULT_THEME_MODE)
		this.#preset = read(THEME_PRESET_KEY, isThemePreset, DEFAULT_THEME_PRESET)
		this.#prefersDark = window.matchMedia(DARK_QUERY).matches
	}

	get mode(): ThemeMode {
		return this.#mode
	}

	set mode(value: ThemeMode) {
		this.#mode = value
		store(THEME_MODE_KEY, value)
		this.apply()
	}

	get preset(): ThemePreset {
		return this.#preset
	}

	set preset(value: ThemePreset) {
		this.#preset = value
		store(THEME_PRESET_KEY, value)
		this.apply()
	}

	/** `dark` or `light`, with `system` already asked. */
	get resolved(): ResolvedTheme {
		if (this.#mode === 'system') return this.#prefersDark ? 'dark' : 'light'
		return this.#mode
	}

	/** Write the current choice onto `<html>`. Idempotent. */
	apply(): void {
		if (typeof document === 'undefined') return
		const el = document.documentElement
		el.classList.toggle('dark', this.resolved === 'dark')
		el.dataset.theme = this.#preset
		// Native form controls, scrollbars and the canvas behind the page.
		el.style.colorScheme = this.resolved
	}

	/** Adopt what another tab (or the other app) stored, without echoing it back. */
	adopt(mode: ThemeMode, preset: ThemePreset): void {
		this.#mode = mode
		this.#preset = preset
		this.apply()
	}

	setPrefersDark(value: boolean): void {
		this.#prefersDark = value
		if (this.#mode === 'system') this.apply()
	}
}

export const theme = new ThemeState()

/**
 * Bind the store to the document: apply what was stored, follow the OS while
 * the mode is `system`, and follow the other app's choice across the
 * `storage` event. Call once from the root layout; returns a teardown for
 * completeness (the root layout never unmounts).
 */
export function initTheme(): () => void {
	if (typeof window === 'undefined') return () => undefined

	theme.apply()

	const media = window.matchMedia(DARK_QUERY)
	const onPreferenceChange = (event: MediaQueryListEvent) => theme.setPrefersDark(event.matches)
	media.addEventListener('change', onPreferenceChange)

	const onStorage = (event: StorageEvent) => {
		if (event.key !== null && event.key !== THEME_MODE_KEY && event.key !== THEME_PRESET_KEY) return
		theme.adopt(
			read(THEME_MODE_KEY, isThemeMode, DEFAULT_THEME_MODE),
			read(THEME_PRESET_KEY, isThemePreset, DEFAULT_THEME_PRESET),
		)
	}
	window.addEventListener('storage', onStorage)

	return () => {
		media.removeEventListener('change', onPreferenceChange)
		window.removeEventListener('storage', onStorage)
	}
}

/**
 * The no-flash boot script. It runs before the first paint, from an inline
 * `<script>` in each app's `app.html`; `<html>` ships with `class="dark"`
 * so even a blocked script lands on the dark default. Keep the copy in
 * `app.html` identical to this string.
 */
export const themeBootScript = `(() => {
	try {
		var el = document.documentElement;
		var mode = localStorage.getItem('${THEME_MODE_KEY}') || '${DEFAULT_THEME_MODE}';
		var preset = localStorage.getItem('${THEME_PRESET_KEY}') || '${DEFAULT_THEME_PRESET}';
		var dark = mode === 'dark' || (mode === 'system' && matchMedia('${DARK_QUERY}').matches);
		el.classList.toggle('dark', dark);
		el.dataset.theme = preset;
		el.style.colorScheme = dark ? 'dark' : 'light';
	} catch (error) {
		// Storage can be unavailable; the markup default (dark) stands.
	}
})();`
