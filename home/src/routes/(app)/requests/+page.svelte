<script lang="ts">
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { createQuery } from '@tanstack/svelte-query';
	import InboxIcon from '@lucide/svelte/icons/inbox';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import { BookRequestsDocument } from '$lib/graphql/generated/graphql';
	import { getHomeSession } from '$lib/session.svelte';
	import { BOOK_REQUEST_STATUS_VALUES, REQUEST_STATUS_LABELS } from '$lib/requests';
	import RequestSummaryCard from '$lib/components/requests/RequestSummaryCard.svelte';

	const session = getHomeSession();
	const canManage = $derived(
		Boolean(
			session.user?.isServerOwner ||
				session.user?.permissions.includes('MANAGE_SERVER') ||
				session.user?.permissions.includes('MANAGE_LIBRARY')
		)
	);
	let mineOnly = $state(false);
	let statusFilter = $state('');

	const requestsQuery = createQuery(() => ({
		queryKey: ['book-requests', mineOnly, statusFilter],
		queryFn: () =>
			request(BookRequestsDocument, {
				status: statusFilter || null,
				mineOnly,
				limit: 50,
				offset: 0
			} as never),
		enabled: browser
	}));
	const requests = $derived(requestsQuery.data?.bookRequests ?? []);
	const statusOptions = $derived(
		Object.entries(REQUEST_STATUS_LABELS).filter(([status]) =>
			BOOK_REQUEST_STATUS_VALUES.includes(status as (typeof BOOK_REQUEST_STATUS_VALUES)[number])
		)
	);
</script>

<svelte:head>
	<title>Requests · Coppice</title>
	<meta name="description" content="Submit book requests and follow their review status." />
</svelte:head>

<div class="flex flex-col gap-6">
	<div class="flex flex-wrap items-start gap-3">
		<div class="mr-auto">
			<h1 class="text-2xl font-semibold tracking-tight">Requests</h1>
			<p class="mt-1 max-w-2xl text-sm text-muted-foreground">
				Submit metadata, follow review decisions, and manage requests you are permitted to see.
			</p>
		</div>
		<Button href={resolve('/requests/new')}>
			<PlusIcon data-icon="inline-start" aria-hidden="true" />
			New request
		</Button>
	</div>

	<div class="flex flex-wrap items-center gap-3 rounded-xl border bg-card p-3">
		<label class="flex items-center gap-2 text-sm">
			<span class="text-muted-foreground">Status</span>
			<select bind:value={statusFilter} class="h-9 rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
				<option value="">All statuses</option>
				{#each statusOptions as [value, label] (value)}
					<option {value}>{label}</option>
				{/each}
			</select>
		</label>
		{#if canManage}
			<label class="flex items-center gap-2 text-sm">
				<input type="checkbox" bind:checked={mineOnly} class="size-4 rounded border-input accent-primary" />
				<span>Only my requests</span>
			</label>
		{:else}
			<span class="text-xs text-muted-foreground">You see your own requests and any explicitly shared request.</span>
		{/if}
		<Button class="ml-auto" variant="ghost" size="sm" onclick={() => requestsQuery.refetch()} disabled={requestsQuery.isFetching}>
			<RefreshCwIcon data-icon="inline-start" aria-hidden="true" class={requestsQuery.isFetching ? 'animate-spin' : undefined} />
			Refresh
		</Button>
	</div>

	{#if requestsQuery.isPending}
		<div class="grid gap-4 md:grid-cols-2">
			{#each { length: 4 } as _, index (index)}
				<Skeleton class="h-36 rounded-xl" />
			{/each}
		</div>
	{:else if requestsQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load requests</AlertTitle>
			<AlertDescription>
				{requestsQuery.error instanceof Error ? requestsQuery.error.message : 'The request service returned an error.'}
			</AlertDescription>
		</Alert>
	{:else if requests.length === 0}
		<Empty class="rounded-xl border border-dashed bg-card">
			<EmptyHeader>
				<EmptyTitle>No requests yet</EmptyTitle>
				<EmptyDescription>Submit a title from this library or hand off metadata from a trusted recommendation.</EmptyDescription>
			</EmptyHeader>
			<div class="flex justify-center">
				<Button href={resolve('/requests/new')}>
					<InboxIcon data-icon="inline-start" aria-hidden="true" />
					Create the first request
				</Button>
			</div>
		</Empty>
	{:else}
		<div class="grid gap-4 md:grid-cols-2">
			{#each requests as item (item.id)}
				<RequestSummaryCard request={item} />
			{/each}
		</div>
	{/if}
</div>
