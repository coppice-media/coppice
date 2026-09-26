import pluginJs from '@eslint/js'
import eslintConfigPrettier from 'eslint-config-prettier'
import simpleImportSort from 'eslint-plugin-simple-import-sort'
import keySort from 'eslint-plugin-sort-keys-fix'
import globals from 'globals'
import tseslint from 'typescript-eslint'

export default [
	{
		ignores: [
			'**/dist/*',
			'**/dev-dist/**',
			'**/target/**',
			'**/.next/**',
			'**/.vercel/**',
		],
	},
	{
		files: ['**/*.{js,mjs,cjs,ts}'],
		plugins: {
			'simple-import-sort': simpleImportSort,
			'sort-keys-fix': keySort,
		},
		rules: {
			'no-console': ['error', { allow: ['warn', 'error'] }],
			'simple-import-sort/imports': 'error',
			'simple-import-sort/exports': 'error',
			'sort-imports': 'off',
			semi: 0,
		},
	},
	{ languageOptions: { globals: globals.node } },
	{ languageOptions: { globals: globals.browser } },
	pluginJs.configs.recommended,
	...tseslint.configs.recommended,
	eslintConfigPrettier,
	{
		files: ['**/*.config.js'],
		rules: {
			'@typescript-eslint/no-require-imports': 'off',
		},
	},
	{
		files: ['scripts/**/*.{js,ts,mjs}'],
		rules: {
			'@typescript-eslint/no-require-imports': 'off',
			'no-console': 'off',
		},
	},
]
