<script lang="ts">
	import type { DeviceEndpointFieldsFragment } from '$lib/graphql/generated/graphql';
	import CopyButton from './CopyButton.svelte';
	import QrCode from './QrCode.svelte';

	let { endpoint }: { endpoint: DeviceEndpointFieldsFragment } = $props();

	const rows = $derived(
		[
			{ label: 'URL', value: endpoint.url, what: 'URL' },
			endpoint.username ? { label: 'Username', value: endpoint.username, what: 'username' } : null,
			{ label: 'Secret', value: endpoint.secretHint, what: 'secret' }
		].filter((row) => row !== null)
	);
</script>

<div class="flex flex-col gap-4 rounded-xl border bg-card p-4 sm:flex-row sm:items-start">
	<QrCode value={endpoint.url} label={`QR code for the ${endpoint.label} URL`} size={112} />
	<dl class="flex min-w-0 flex-1 flex-col gap-1.5 text-sm">
		<div class="mb-1 font-medium">{endpoint.label}</div>
		{#each rows as row (row.label)}
			<div class="flex items-center gap-2">
				<dt class="w-18 shrink-0 text-xs text-muted-foreground">{row.label}</dt>
				<dd class="min-w-0 flex-1 truncate font-mono text-xs" title={row.value}>{row.value}</dd>
				<CopyButton value={row.value} what={row.what} size="icon-xs" />
			</div>
		{/each}
	</dl>
</div>
