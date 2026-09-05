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
		adapter: adapter({ pages: 'build', assets: 'build', fallback: 'index.html' }),
		// The headless server mounts the built Home app under `/app`
		// (`STUMP_HOME_APP_DIR`); override for other deployments.
		paths: { base: process.env.HOME_BASE_PATH ?? '/app' }
	}
};

export default config;
