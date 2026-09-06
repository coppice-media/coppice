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
		// tree at src/lib/stump-ui; `preserveSymlinks` in tsconfig/vite keeps
		// its files inside this project so they typecheck as first-class
		// sources and resolve their dependencies from this app's node_modules.
		alias: {
			'@stump/ui': 'src/lib/stump-ui'
		},
		adapter: adapter({ pages: 'build', assets: 'build', fallback: 'index.html' }),
		// The headless server mounts the built Home app under `/app`
		// (`STUMP_HOME_APP_DIR`); override for other deployments.
		paths: { base: process.env.HOME_BASE_PATH ?? '/app' }
	}
};

export default config;
