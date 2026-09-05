// Ambient declarations for SvelteKit's virtual `$app/*` modules, so this
// package typechecks standalone. At runtime the consuming SvelteKit app
// resolves the real implementations (the Kit Vite pipeline aliases `$app/*`
// for every module it compiles); these declarations only satisfy the checker
// and deliberately mirror the subset of the API this package uses.

declare module '$app/environment' {
	export const browser: boolean;
	export const dev: boolean;
}

declare module '$app/paths' {
	export const base: string;
	export const assets: string;
	export function resolve(
		routeId: string,
		params?: Record<string, string | undefined>
	): string;
}
