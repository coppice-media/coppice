import { existsSync, readFileSync } from 'node:fs';
import type { CodegenConfig } from '@graphql-codegen/cli';

const repositorySchema = '../crates/graphql/schema.graphql';
const hasIngestSchema = existsSync(repositorySchema)
	&& readFileSync(repositorySchema, 'utf8').includes('type IngestDropItem');

const config: CodegenConfig = {
	schema: hasIngestSchema ? repositorySchema : [repositorySchema, 'src/lib/graphql/ingest.graphql'],
	documents: ['src/lib/graphql/**/*.graphql', '!src/lib/graphql/ingest.graphql'],
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
					JSON: 'unknown',
					Upload: 'File'
				},
				useTypeImports: true
			}
		}
	}
};

export default config;
