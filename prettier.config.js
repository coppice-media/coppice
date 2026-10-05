module.exports = {
	plugins: ['prettier-plugin-tailwindcss'],
	tailwindFunctions: ['cn'],
	printWidth: 100,
	semi: false,
	singleQuote: true,
	tabWidth: 2,
	tailwindStylesheet: './home/src/routes/layout.css',
	trailingComma: 'all',
	useTabs: true,
	overrides: [
		{
			files: 'docs/**/*.{ts,tsx,mdx}',
			options: { tailwindStylesheet: './docs/src/styles/app.css' },
		},
	],
}
