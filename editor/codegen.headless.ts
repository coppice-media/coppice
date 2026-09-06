// Throwaway: validate the editor's documents against a headless-built SDL
// (graphql without `web`, without `providers`). Delete after the check.
import type { CodegenConfig } from '@graphql-codegen/cli';

const config: CodegenConfig = {
	schema: '/tmp/sdl-noweb.graphql',
	documents: ['src/lib/graphql/**/*.graphql', '!src/lib/graphql/ingest.graphql'],
	generates: {
		'/tmp/editor-headless-out/': {
			preset: 'client',
			presetConfig: {
				fragmentMasking: false
			},
			config: {
				enumsAsTypes: true,
				scalars: {
					DateTime: 'string',
					JSON: 'unknown',
					Upload: 'File'
				},
				useTypeImports: true
			}
		}
	}
};

export default config;
