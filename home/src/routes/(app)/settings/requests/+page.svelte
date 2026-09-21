<script lang="ts">
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import LockKeyholeIcon from '@lucide/svelte/icons/lock-keyhole';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import { BookRequestGatewayDocument, UpdateBookRequestGatewayDocument } from '$lib/graphql/generated/graphql';
	import { getHomeSession } from '$lib/session.svelte';
	import RequestGatewayForm from '$lib/components/requests/RequestGatewayForm.svelte';

	const session = getHomeSession();
	const canManage = $derived(Boolean(session.user?.isServerOwner || session.user?.permissions.includes('MANAGE_SERVER')));
	const queryClient = useQueryClient();
	const gatewayQuery = createQuery(() => ({
		queryKey: ['request-gateway'],
		queryFn: () => request(BookRequestGatewayDocument, {}),
		enabled: browser && canManage
	}));
	const gateway = $derived(gatewayQuery.data?.bookRequestGateway ?? null);

	const saveGateway = createMutation(() => ({
		mutationFn: (input: unknown) => request(UpdateBookRequestGatewayDocument, { input } as never),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['request-gateway'] });
		},
		onError: () => undefined
	}));

	function save(input: Record<string, unknown>): void {
		// The backend accepts this redacted marker to preserve an existing token.
		// No plaintext secret is ever retained in component state after submit.
		saveGateway.mutate({ ...input, token: input.token || '********' });
	}
</script>

<svelte:head>
	<title>Request policy · Coppice</title>
	<meta name="description" content="Configure owner-only approval and private request gateway policy." />
</svelte:head>

<div class="flex flex-col gap-6">
	<div class="flex flex-wrap items-start gap-3">
		<div class="mr-auto">
			<Button variant="ghost" size="sm" href={resolve('/connections')}>
				<ArrowLeftIcon data-icon="inline-start" aria-hidden="true" />
				Connections
			</Button>
			<h1 class="mt-3 text-2xl font-semibold tracking-tight">Request policy</h1>
			<p class="mt-1 max-w-2xl text-sm text-muted-foreground">Administrator-only controls for approval, source scoring, retries, and the private gateway connection.</p>
		</div>
	</div>

	{#if !canManage}
		<Alert>
			<LockKeyholeIcon class="size-4" aria-hidden="true" />
			<AlertTitle>Administrator access required</AlertTitle>
			<AlertDescription>Gateway credentials and request policy are server-wide settings. Ask a server administrator to configure them.</AlertDescription>
		</Alert>
	{:else if gatewayQuery.isPending}
		<Skeleton class="h-[43rem] rounded-xl" />
	{:else if gatewayQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load request policy</AlertTitle>
			<AlertDescription>{gatewayQuery.error instanceof Error ? gatewayQuery.error.message : 'The server could not report gateway settings.'}</AlertDescription>
		</Alert>
	{:else}
		<RequestGatewayForm
			settings={gateway}
			busy={saveGateway.isPending}
			error={saveGateway.error instanceof Error ? saveGateway.error.message : null}
			onsave={save}
		/>
	{/if}
</div>
