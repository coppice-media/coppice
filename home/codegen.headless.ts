// Throwaway: validate the home app's documents against a headless-built SDL
// (graphql without `web`, without `providers`). Delete after the check.
import type { CodegenConfig } from '@graphql-codegen/cli';

const config: CodegenConfig = {
	schema: '/tmp/sdl-noweb.graphql',
	documents: ['src/lib/graphql/**/*.graphql'],
	generates: {
		'/tmp/home-headless-out/': {
			preset: 'client',
			presetConfig: {
				fragmentMasking: false
			},
			config: {
				enumsAsTypes: true,
				scalars: {
					DateTime: 'string',
					NaiveDate: 'string',
					JSON: 'unknown',
					Upload: 'File'
				},
				useTypeImports: true
			}
		}
	}
};

export default config;
