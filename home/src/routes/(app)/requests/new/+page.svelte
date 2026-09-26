<script lang="ts">
	import { browser } from '$app/environment';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { createMutation, createQuery } from '@tanstack/svelte-query';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import { Button } from '@stump/ui/components/ui/button';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import { CreateBookRequestDocument, RequestDestinationsDocument, type RequestFormat } from '$lib/graphql/generated/graphql';
	import { externalReferenceFromParams, queryText, safeCoverUrl } from '$lib/requests';
	import RequestCreateForm from '$lib/components/requests/RequestCreateForm.svelte';
	const params = $derived(page.url.searchParams);
	const recommendationId = $derived(params.get('recommendationId') ?? '');
	const targetId = $derived(params.get('targetId') ?? '');
	const external = $derived(externalReferenceFromParams(params));
	const initialMediaId = $derived(params.get('mediaId') ?? '');
	const initialWorkId = $derived(params.get('workId') ?? '');
	const initialMode = $derived(external || (!initialMediaId && !initialWorkId) ? 'external' : 'internal');
	const destinationShelfId = $derived(params.get('destinationShelfId') ?? '');
	const destinationDeviceId = $derived(params.get('destinationDeviceId') ?? '');
	// An Audible hit hands off its edition's format and narrator with the metadata.
	const initialFormat = $derived<RequestFormat>(
		params.get('format') === 'AUDIOBOOK' ? 'AUDIOBOOK' : params.get('format') === 'EBOOK' ? 'EBOOK' : 'ANY'
	);
	const initialNarrator = $derived(queryText(params, 'narrator'));

	const destinationsQuery = createQuery(() => ({
		queryKey: ['request-destinations'],
		queryFn: () => request(RequestDestinationsDocument, {}),
		enabled: browser
	}));
	const devices = $derived(
		(destinationsQuery.data?.devices ?? []).filter((device) => !device.revokedAt).map((device) => ({ id: device.id, name: device.name }))
	);
	const shelves = $derived((destinationsQuery.data?.readingLists?.nodes ?? []).map((shelf) => ({ id: shelf.id, name: shelf.name })));

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
			<p class="mt-1 max-w-2xl text-sm text-muted-foreground">Submit metadata and an optional destination for review. Coppice keeps the request and its approval history.</p>
		</div>
	</div>

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
			{initialFormat}
			{initialNarrator}
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
