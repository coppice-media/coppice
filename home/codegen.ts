import type { CodegenConfig } from '@graphql-codegen/cli';

const config: CodegenConfig = {
	schema: '../crates/graphql/schema.graphql',
	documents: ['src/lib/graphql/**/*.graphql'],
	generates: {
		'src/lib/graphql/generated/': {
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
