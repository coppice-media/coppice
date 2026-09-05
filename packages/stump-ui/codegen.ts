import type { CodegenConfig } from '@graphql-codegen/cli';

const config: CodegenConfig = {
	schema: '../../crates/graphql/schema.graphql',
	documents: ['src/graphql/**/*.graphql'],
	generates: {
		'src/graphql/generated/': {
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
