<script lang="ts">
	import type { IssuedCredential } from '$lib/devices';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import CopyButton from './CopyButton.svelte';
	import EndpointCard from './EndpointCard.svelte';

	let { issued }: { issued: IssuedCredential } = $props();
</script>

<div class="flex flex-col gap-4">
	<Alert>
		<AlertTitle>Shown once</AlertTitle>
		<AlertDescription>
			The secret for <strong>{issued.device.name}</strong> is only shown now. Configure the client
			before closing this dialog; you can rotate the credential later to get a new one.
		</AlertDescription>
	</Alert>
	<div class="flex items-center gap-1 rounded-lg border bg-muted/40 p-3 text-sm">
		<span class="w-20 shrink-0 text-muted-foreground">Secret</span>
		<code class="min-w-0 flex-1 truncate font-mono text-xs">{issued.credential.secret}</code>
		<CopyButton value={issued.credential.secret} what="secret" />
	</div>
	{#if issued.endpoints.length}
		<div class="flex flex-col gap-3">
			<h3 class="text-sm font-medium">Where to point the client</h3>
			{#each issued.endpoints as endpoint (endpoint.label + endpoint.url)}
				<EndpointCard {endpoint} />
			{/each}
		</div>
	{:else}
		<p class="text-sm text-muted-foreground">
			This client kind has no fixed endpoint; use the secret as an API key against this server's
			origin.
		</p>
	{/if}
</div>
