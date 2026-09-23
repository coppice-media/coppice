import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	preprocess: vitePreprocess(),
	compilerOptions: {
		// Runes everywhere except third-party components (drop in Svelte 6).
		runes: ({ filename }) => (filename.split(/[/\\]/).includes('node_modules') ? undefined : true)
	},
	kit: {
		// The shared UI foundation (packages/stump-ui/src) is linked into the
		// tree at src/lib/stump-ui; Vite realpaths that source while deduping
		// Svelte, so isolated Bun installs retain package dependency ownership.
		alias: {
			'@stump/ui': 'src/lib/stump-ui'
		},
		adapter: adapter({ pages: 'build', assets: 'build', fallback: 'index.html' }),
		// The headless server mounts the built editor under `/editor`
		// (`INGEST_EDITOR_DIR`); override for other deployments.
		paths: { base: process.env.EDITOR_BASE_PATH ?? '/editor' }
	}
};

export default config;
