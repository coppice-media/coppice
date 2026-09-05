<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';

	let { children } = $props();

	const tabs = [
		{ href: resolve('/settings/providers'), label: 'Ingest providers' },
		{ href: resolve('/settings/keys'), label: 'API keys' }
	];

	function isActive(href: string): boolean {
		return page.url.pathname === href || page.url.pathname.startsWith(`${href}/`);
	}
</script>

<div class="flex flex-col gap-6">
	<nav aria-label="Settings navigation" class="flex w-fit gap-1 rounded-lg bg-muted p-1 text-sm">
		{#each tabs as tab (tab.href)}
			<a
				class="rounded-md px-3 py-1.5 {isActive(tab.href)
					? 'bg-background font-medium shadow-sm'
					: 'text-muted-foreground hover:text-foreground'}"
				href={tab.href}
				aria-current={isActive(tab.href) ? 'page' : undefined}
			>
				{tab.label}
			</a>
		{/each}
	</nav>
	{@render children()}
</div>
