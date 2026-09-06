<script lang="ts">
	import type { DeviceEndpointFieldsFragment } from '$lib/graphql/generated/graphql';
	import CopyButton from './CopyButton.svelte';
	import QrCode from './QrCode.svelte';

	let { endpoint }: { endpoint: DeviceEndpointFieldsFragment } = $props();
</script>

<div class="flex flex-col gap-3 rounded-lg border bg-card p-4 sm:flex-row sm:items-start">
	<QrCode value={endpoint.url} label={`QR code for the ${endpoint.label} URL`} size={128} />
	<dl class="min-w-0 flex-1 text-sm">
		<div class="mb-2 font-medium">{endpoint.label}</div>
		<div class="flex items-center gap-1">
			<dt class="w-20 shrink-0 text-muted-foreground">URL</dt>
			<dd class="min-w-0 flex-1 truncate font-mono text-xs" title={endpoint.url}>{endpoint.url}</dd>
			<CopyButton value={endpoint.url} what="URL" />
		</div>
		{#if endpoint.username}
			<div class="flex items-center gap-1">
				<dt class="w-20 shrink-0 text-muted-foreground">Username</dt>
				<dd class="min-w-0 flex-1 truncate font-mono text-xs">{endpoint.username}</dd>
				<CopyButton value={endpoint.username} what="username" />
			</div>
		{/if}
		<div class="flex items-center gap-1">
			<dt class="w-20 shrink-0 text-muted-foreground">Secret</dt>
			<dd class="min-w-0 flex-1 truncate font-mono text-xs">{endpoint.secretHint}</dd>
			<CopyButton value={endpoint.secretHint} what="secret" />
		</div>
	</dl>
</div>
