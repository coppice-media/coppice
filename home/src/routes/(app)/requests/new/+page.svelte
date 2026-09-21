<script lang="ts">
	import { browser } from '$app/environment';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { createMutation, createQuery } from '@tanstack/svelte-query';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import CheckCircle2Icon from '@lucide/svelte/icons/check-circle-2';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import { BookRequestGatewayDocument, CreateBookRequestDocument, RequestDestinationsDocument } from '$lib/graphql/generated/graphql';
	import { getHomeSession } from '$lib/session.svelte';
	import { externalReferenceFromParams, safeCoverUrl } from '$lib/requests';
	import RequestCreateForm from '$lib/components/requests/RequestCreateForm.svelte';
	const session = getHomeSession();
	const canManage = $derived(Boolean(session.user?.isServerOwner || session.user?.permissions.includes('MANAGE_SERVER')));
	const params = $derived(page.url.searchParams);
	const recommendationId = $derived(params.get('recommendationId') ?? '');
	const targetId = $derived(params.get('targetId') ?? '');
	const external = $derived(externalReferenceFromParams(params));
	const initialMediaId = $derived(params.get('mediaId') ?? '');
	const initialWorkId = $derived(params.get('workId') ?? '');
	const initialMode = $derived(external || (!initialMediaId && !initialWorkId) ? 'external' : 'internal');
	const destinationShelfId = $derived(params.get('destinationShelfId') ?? '');
	const destinationDeviceId = $derived(params.get('destinationDeviceId') ?? '');

	const destinationsQuery = createQuery(() => ({
		queryKey: ['request-destinations'],
		queryFn: () => request(RequestDestinationsDocument, {}),
		enabled: browser
	}));
	const gatewayQuery = createQuery(() => ({
		queryKey: ['request-gateway'],
		queryFn: () => request(BookRequestGatewayDocument, {}),
		enabled: browser && canManage
	}));
	const devices = $derived(
		(destinationsQuery.data?.devices ?? []).filter((device) => !device.revokedAt).map((device) => ({ id: device.id, name: device.name }))
	);
	const shelves = $derived((destinationsQuery.data?.readingLists?.nodes ?? []).map((shelf) => ({ id: shelf.id, name: shelf.name })));
	const gateway = $derived(gatewayQuery.data?.bookRequestGateway ?? null);
	const gatewayReady = $derived(Boolean(gateway?.enabled && gateway.hasToken));

	const createRequest = createMutation(() => ({
		mutationFn: (input: unknown) => request(CreateBookRequestDocument, { input } as never),
		onSuccess: (result) => {
			void goto(`${resolve('/requests')}/${result.createBookRequest.id}`);
		},
		onError: () => undefined
	}));

	function submit(input: unknown): void {
		createRequest.mutate(input);
	}
</script>

<svelte:head>
	<title>New request · Coppice</title>
</svelte:head>

<div class="flex flex-col gap-6">
	<div class="flex flex-wrap items-start gap-3">
		<div class="mr-auto">
			<Button variant="ghost" size="sm" href={resolve('/requests')}>
				<ArrowLeftIcon data-icon="inline-start" aria-hidden="true" />
				All requests
			</Button>
			<h1 class="mt-3 text-2xl font-semibold tracking-tight">New book request</h1>
			<p class="mt-1 max-w-2xl text-sm text-muted-foreground">Submit metadata first. Approval, source search, release selection, and private import happen on the request detail page.</p>
		</div>
	</div>

	{#if gatewayQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Gateway status unavailable</AlertTitle>
			<AlertDescription>{gatewayQuery.error instanceof Error ? gatewayQuery.error.message : 'The server could not report gateway configuration.'} You can still submit a request for an operator to review.</AlertDescription>
		</Alert>
	{:else if gateway && !gatewayReady}
		<Alert>
			<CircleAlertIcon class="size-4" aria-hidden="true" />
			<AlertTitle>Private gateway is not ready</AlertTitle>
			<AlertDescription>
				Requests can be submitted, but search and download will wait until an owner enables the gateway and stores its token. Secrets remain hidden.
			</AlertDescription>
		</Alert>
	{:else if gatewayReady}
		<Alert>
			<CheckCircle2Icon class="size-4 text-emerald-500" aria-hidden="true" />
			<AlertTitle>Gateway configured</AlertTitle>
			<AlertDescription>Source health is verified when an approved request runs search; this screen never handles tracker credentials.</AlertDescription>
		</Alert>
	{/if}

	{#if destinationsQuery.isPending}
		<Skeleton class="h-[38rem] rounded-xl" />
	{:else}
		<RequestCreateForm
			initialMode={initialMode}
			initialMediaId={initialMediaId}
			initialWorkId={initialWorkId}
			initialProvider={external?.sourceProvider ?? params.get('provider') ?? ''}
			initialRemoteId={external?.remoteId ?? params.get('remoteId') ?? ''}
			initialExternalKey={external?.externalKey ?? params.get('externalKey') ?? ''}
			initialTitle={external?.title ?? params.get('title') ?? ''}
			initialAuthors={external?.authors ?? params.get('authors') ?? ''}
			initialCoverUrl={safeCoverUrl(external?.coverUrl ?? params.get('coverUrl')) ?? ''}
			initialDestinationShelfId={destinationShelfId}
			initialDestinationDeviceId={destinationDeviceId}
			{recommendationId}
			{targetId}
			{devices}
			{shelves}
			busy={createRequest.isPending}
			error={createRequest.error instanceof Error ? createRequest.error.message : null}
			oncreate={submit}
		/>
	{/if}
</div>
