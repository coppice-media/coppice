<script lang="ts">
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import DownloadIcon from '@lucide/svelte/icons/download';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import SearchIcon from '@lucide/svelte/icons/search';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import XCircleIcon from '@lucide/svelte/icons/x-circle';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import * as AlertDialog from '@stump/ui/components/ui/alert-dialog';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import {
		ApproveBookRequestDocument,
		BookRequestDocument,
		BookRequestReleasesDocument,
		GrabBookRequestDocument,
		PollBookRequestGrabDocument,
		RejectBookRequestDocument,
		RequestDestinationsDocument,
		RetryBookRequestDocument,
		SearchBookRequestDocument,
		SelectBookRequestReleaseDocument
	} from '$lib/graphql/generated/graphql';
	import { getHomeSession } from '$lib/session.svelte';
	import { absoluteTime, relativeTime } from '$lib/format';
	import { safeCoverUrl, statusDescription } from '$lib/requests';
	import RequestReleasePicker from '$lib/components/requests/RequestReleasePicker.svelte';
	import RequestProgress from '$lib/components/requests/RequestProgress.svelte';
	import RequestStatusBadge from '$lib/components/requests/RequestStatusBadge.svelte';

	const queryClient = useQueryClient();
	const session = getHomeSession();
	const requestId = $derived(page.params.id ?? '');
	const canManage = $derived(
		Boolean(
			session.user?.isServerOwner ||
				session.user?.permissions.includes('MANAGE_SERVER') ||
				session.user?.permissions.includes('MANAGE_LIBRARY')
		)
	);

	const detailQuery = createQuery(() => ({
		queryKey: ['book-request', requestId],
		queryFn: () => request(BookRequestDocument, { id: requestId }),
		enabled: browser && !!requestId
	}));
	const requestRecord = $derived(detailQuery.data?.bookRequest ?? null);
	const requestStatus = $derived(requestRecord?.status ?? '');
	const releasesQuery = createQuery(() => ({
		queryKey: ['book-request-releases', requestId],
		queryFn: () => request(BookRequestReleasesDocument, { requestId }),
		enabled: browser && !!requestId && !['PENDING', 'AWAITING_APPROVAL', 'REJECTED'].includes(requestStatus)
	}));
	const releases = $derived(releasesQuery.data?.bookRequestReleases ?? []);
	const destinationsQuery = createQuery(() => ({
		queryKey: ['request-destinations'],
		queryFn: () => request(RequestDestinationsDocument, {}),
		enabled: browser && !!requestId
	}));
	const shelfName = $derived(
		requestRecord?.destinationShelfId
			? destinationsQuery.data?.readingLists?.nodes.find((shelf) => shelf.id === requestRecord.destinationShelfId)?.name ?? requestRecord.destinationShelfId
			: null
	);
	const deviceName = $derived(
		requestRecord?.destinationDeviceId
			? destinationsQuery.data?.devices.find((device) => device.id === requestRecord.destinationDeviceId)?.name ?? requestRecord.destinationDeviceId
			: null
	);

	let actionError = $state<string | null>(null);
	let decisionReason = $state('');
	let grabConfirmOpen = $state(false);
	let grab = $state<{
		id: string;
		status: string;
		attempts: number;
		maxAttempts: number;
		failureCode?: string | null;
		failureMessage?: string | null;
		lastPolledAt?: string | null;
		finishedAt?: string | null;
	} | null>(null);

	function invalidate(): void {
		void queryClient.invalidateQueries({ queryKey: ['book-request', requestId] });
		void queryClient.invalidateQueries({ queryKey: ['book-request-releases', requestId] });
		void queryClient.invalidateQueries({ queryKey: ['book-requests'] });
	}

	function showError(error: unknown, fallback: string): void {
		actionError = error instanceof Error ? error.message : fallback;
	}

	const approve = createMutation(() => ({
		mutationFn: () => request(ApproveBookRequestDocument, { requestId, reason: decisionReason.trim() || null } as never),
		onSuccess: () => {
			actionError = null;
			decisionReason = '';
			invalidate();
		},
		onError: (error) => showError(error, 'Approval failed.')
	}));
	const reject = createMutation(() => ({
		mutationFn: () => request(RejectBookRequestDocument, { requestId, reason: decisionReason.trim() || null } as never),
		onSuccess: () => {
			actionError = null;
			decisionReason = '';
			invalidate();
		},
		onError: (error) => showError(error, 'Declining the request failed.')
	}));
	const search = createMutation(() => ({
		mutationFn: () => request(SearchBookRequestDocument, { requestId }),
		onSuccess: () => {
			actionError = null;
			invalidate();
			void releasesQuery.refetch();
		},
		onError: (error) => showError(error, 'Source search failed. Check gateway health and retry.')
	}));
	const selectRelease = createMutation(() => ({
		mutationFn: (releaseId: string) => request(SelectBookRequestReleaseDocument, { requestId, releaseId }),
		onSuccess: () => {
			actionError = null;
			invalidate();
		},
		onError: (error) => showError(error, 'The release could not be selected.')
	}));
	const startGrab = createMutation(() => ({
		mutationFn: () => request(GrabBookRequestDocument, { requestId }),
		onSuccess: (result) => {
			actionError = null;
			grab = result.grabBookRequest;
			grabConfirmOpen = false;
			invalidate();
		},
		onError: (error) => showError(error, 'The gateway did not accept this grab.')
	}));
	const pollGrab = createMutation(() => ({
		mutationFn: (grabId: string) => request(PollBookRequestGrabDocument, { grabId }),
		onSuccess: (result) => {
			grab = result.pollBookRequestGrab;
			if (['COMPLETE', 'COMPLETED', 'DOWNLOADED', 'FAILED', 'ERROR', 'EXPIRED'].includes(result.pollBookRequestGrab.status.toUpperCase())) invalidate();
		},
		onError: (error) => showError(error, 'Download status could not be refreshed.')
	}));
	const retry = createMutation(() => ({
		mutationFn: () => request(RetryBookRequestDocument, { requestId }),
		onSuccess: () => {
			actionError = null;
			grab = null;
			invalidate();
		},
		onError: (error) => showError(error, 'Retry was not authorized or the request is no longer retryable.')
	}));

	const selectedRelease = $derived(releases.find((release) => release.selected) ?? null);
	const canApprove = $derived(canManage && ['PENDING', 'AWAITING_APPROVAL'].includes(requestStatus));
	const canSearch = $derived(requestStatus === 'APPROVED' && (canManage || requestRecord?.requesterId === session.user?.id));
	const canSelect = $derived(['SEARCHING', 'APPROVED', 'AWAITING_APPROVAL', 'NEEDS_SELECTION'].includes(requestStatus) && releases.length > 0);
	const canGrab = $derived(Boolean(selectedRelease && requestStatus === 'APPROVED'));
	const canRetry = $derived(requestStatus === 'FAILED' && (requestRecord?.retries ?? 0) < (requestRecord?.maxRetries ?? 0));
	const grabTerminal = $derived(Boolean(grab && ['COMPLETE', 'COMPLETED', 'DOWNLOADED', 'FAILED', 'ERROR', 'EXPIRED'].includes(grab.status.toUpperCase())));

	$effect(() => {
		if (!grab || grabTerminal || pollGrab.isPending) return;
		const timer = setInterval(() => pollGrab.mutate(grab?.id ?? ''), 8000);
		return () => clearInterval(timer);
	});

	function openGrabConfirmation(): void {
		if (canGrab) grabConfirmOpen = true;
	}

	function confirmGrab(): void {
		if (!canGrab || startGrab.isPending) return;
		startGrab.mutate();
	}
</script>

<svelte:head>
	<title>{requestRecord ? `${requestRecord.title} · Request` : 'Request · Coppice'}</title>
</svelte:head>

<div class="flex flex-col gap-6">
	<div class="flex flex-wrap items-start gap-3">
		<div class="mr-auto">
			<Button variant="ghost" size="sm" href={resolve('/requests')}>
				<ArrowLeftIcon data-icon="inline-start" aria-hidden="true" />
				All requests
			</Button>
			{#if requestRecord}
				<div class="mt-3 flex items-start gap-3">
					{#if safeCoverUrl(requestRecord.coverUrl)}
						<img src={safeCoverUrl(requestRecord.coverUrl)} alt="" class="size-16 rounded-md border object-cover" />
					{/if}
					<div>
						<div class="flex flex-wrap items-center gap-2">
							<h1 class="text-2xl font-semibold tracking-tight">{requestRecord.title}</h1>
							<RequestStatusBadge status={requestRecord.status} />
						</div>
						<p class="mt-1 text-sm text-muted-foreground">{requestRecord.authors || 'Author not provided'}{requestRecord.sourceProvider ? ` · ${requestRecord.sourceProvider}` : ''}</p>
					</div>
				</div>
			{/if}
		</div>
		<Button variant="outline" href={resolve('/requests/new')}>New request</Button>
	</div>

	{#if detailQuery.isPending}
		<Skeleton class="h-36 rounded-xl" />
	{:else if detailQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load this request</AlertTitle>
			<AlertDescription>{detailQuery.error instanceof Error ? detailQuery.error.message : 'The request service returned an error.'}</AlertDescription>
		</Alert>
	{:else if !requestRecord}
		<Alert variant="destructive">
			<AlertTitle>Request not found</AlertTitle>
			<AlertDescription>This request may have been removed or is not visible to your account.</AlertDescription>
		</Alert>
	{:else}
		{#if actionError}
			<Alert variant="destructive">
				<AlertTitle>Request action failed</AlertTitle>
				<AlertDescription>{actionError}</AlertDescription>
			</Alert>
		{/if}

		<RequestProgress status={requestRecord.status} message={statusDescription(requestRecord.status)} error={requestRecord.failureMessage} />

		<div class="grid gap-5 xl:grid-cols-[minmax(0,1fr)_22rem]">
			<div class="flex min-w-0 flex-col gap-5">
				{#if requestRecord.status === 'SEARCHING' || releases.length > 0 || releasesQuery.isError}
					{#if releasesQuery.isError}
						<Alert variant="destructive">
							<AlertTitle>Source search results unavailable</AlertTitle>
							<AlertDescription>{releasesQuery.error instanceof Error ? releasesQuery.error.message : 'Retry search to refresh releases.'}</AlertDescription>
						</Alert>
					{:else}
						<RequestReleasePicker
							releases={releases}
							selectedReleaseId={selectedRelease?.id ?? null}
							minimumScore={requestRecord.scoringFloor}
							busy={selectRelease.isPending}
							disabled={!canSelect}
							onconfirm={(release) => selectRelease.mutate(release.id)}
						/>
					{/if}
				{/if}

				{#if selectedRelease}
					<Card>
						<CardHeader>
							<CardTitle class="text-base">Download confirmation</CardTitle>
							<CardDescription>The release is selected but no download starts until you confirm the grab below.</CardDescription>
						</CardHeader>
						<CardContent class="flex flex-wrap items-center justify-between gap-3">
							<div class="min-w-0">
								<p class="truncate font-medium">{selectedRelease.title}</p>
								<p class="text-sm text-muted-foreground">{selectedRelease.sourceProvider} · score {selectedRelease.score}</p>
							</div>
							<Button onclick={openGrabConfirmation} disabled={!canGrab || startGrab.isPending}>
								<DownloadIcon data-icon="inline-start" aria-hidden="true" />
								{startGrab.isPending ? 'Starting…' : 'Confirm download'}
							</Button>
						</CardContent>
					</Card>
				{/if}

				{#if grab}
					<Card>
						<CardHeader>
							<div class="flex flex-wrap items-center gap-2">
								<CardTitle class="text-base">Download and import</CardTitle>
								<Badge variant={grabTerminal && grab.status.toUpperCase() !== 'COMPLETED' ? 'destructive' : 'secondary'}>{grab.status}</Badge>
							</div>
							<CardDescription>Progress comes from the private gateway and then the existing staging/import queue.</CardDescription>
							<div class="mt-3 h-1.5 overflow-hidden rounded-full bg-muted" role="progressbar" aria-label="Download and import progress">
								<div class="h-full rounded-full {grabTerminal && grab.status.toUpperCase() === 'COMPLETED' ? 'w-full bg-emerald-500' : grabTerminal ? 'w-full bg-destructive' : 'w-1/3 animate-pulse bg-primary'}"></div>
							</div>
						</CardHeader>
						<CardContent class="flex flex-col gap-3">
							<div class="grid gap-3 text-sm sm:grid-cols-3">
								<div><span class="text-muted-foreground">Attempts</span><strong class="mt-0.5 block tabular-nums">{grab.attempts}/{grab.maxAttempts}</strong></div>
								<div><span class="text-muted-foreground">Last checked</span><strong class="mt-0.5 block">{relativeTime(grab.lastPolledAt)}</strong></div>
								<div><span class="text-muted-foreground">Finished</span><strong class="mt-0.5 block">{absoluteTime(grab.finishedAt)}</strong></div>
							</div>
							{#if grab.failureMessage}
								<p class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive" role="alert">{grab.failureCode ? `${grab.failureCode}: ` : ''}{grab.failureMessage}</p>
							{/if}
							{#if !grabTerminal}
								<Button variant="outline" size="sm" onclick={() => pollGrab.mutate(grab?.id ?? '')} disabled={pollGrab.isPending}>
									{pollGrab.isPending ? 'Checking…' : 'Check now'}
								</Button>
							{/if}
						</CardContent>
					</Card>
				{/if}
			</div>

			<aside class="flex flex-col gap-5">
				<Card>
					<CardHeader><CardTitle class="text-base">Request details</CardTitle></CardHeader>
					<CardContent class="flex flex-col gap-3 text-sm">
						<div><span class="text-muted-foreground">Created</span><strong class="mt-0.5 block">{absoluteTime(requestRecord.createdAt)}</strong></div>
						<div><span class="text-muted-foreground">Last updated</span><strong class="mt-0.5 block">{relativeTime(requestRecord.updatedAt)}</strong></div>
						<div><span class="text-muted-foreground">Approval policy</span><strong class="mt-0.5 block">{requestRecord.approvalPolicy === 'REQUIRED' ? 'Operator approval required' : 'Approval optional'}</strong></div>
						{#if requestRecord.approvedAt}<div><span class="text-muted-foreground">Approved</span><strong class="mt-0.5 block">{absoluteTime(requestRecord.approvedAt)}</strong></div>{/if}
						{#if requestRecord.failureCode}<div><span class="text-muted-foreground">Failure code</span><strong class="mt-0.5 block font-mono text-destructive">{requestRecord.failureCode}</strong></div>{/if}
						{#if requestRecord.sourceProvider}
							<div><span class="text-muted-foreground">Provider reference</span><strong class="mt-0.5 block break-words">{requestRecord.sourceProvider} · {requestRecord.remoteId}</strong></div>
							{#if requestRecord.externalKey}<div><span class="text-muted-foreground">External key</span><strong class="mt-0.5 block break-words">{requestRecord.externalKey}</strong></div>{/if}
						{/if}
						<div><span class="text-muted-foreground">Score floor</span><strong class="mt-0.5 block tabular-nums">{requestRecord.scoringFloor} / 100</strong></div>
						<div><span class="text-muted-foreground">Verification threshold</span><strong class="mt-0.5 block tabular-nums">{requestRecord.verificationThreshold} / 100</strong></div>
						{#if shelfName || deviceName}
							<div class="border-t pt-3"><span class="text-muted-foreground">After import</span><strong class="mt-0.5 block">{[shelfName, deviceName].filter(Boolean).join(' · ')}</strong></div>
						{/if}
						<div class="border-t pt-3"><span class="text-muted-foreground">Automation</span><strong class="mt-0.5 block">{requestRecord.automationEnabled ? 'Enabled if policy allows' : 'Off'}</strong></div>
					</CardContent>
				</Card>

				{#if canApprove}
					<Card>
						<CardHeader><CardTitle class="text-base">Operator decision</CardTitle><CardDescription>Approval starts no download by itself; it only unlocks source search.</CardDescription></CardHeader>
						<CardContent class="flex flex-col gap-3">
							<div class="flex flex-col gap-2"><Label for="request-decision-reason">Reason <span class="font-normal text-muted-foreground">(optional)</span></Label><Input id="request-decision-reason" bind:value={decisionReason} placeholder="Short note for the requester" /></div>
							<div class="grid gap-2 sm:grid-cols-2">
								<Button onclick={() => approve.mutate()} disabled={approve.isPending || reject.isPending}><ShieldCheckIcon data-icon="inline-start" aria-hidden="true" />{approve.isPending ? 'Approving…' : 'Approve'}</Button>
								<Button variant="destructive" onclick={() => reject.mutate()} disabled={approve.isPending || reject.isPending}><XCircleIcon data-icon="inline-start" aria-hidden="true" />{reject.isPending ? 'Declining…' : 'Decline'}</Button>
							</div>
						</CardContent>
					</Card>
				{/if}

				{#if requestStatus === 'APPROVED'}
					<Card>
						<CardHeader><CardTitle class="text-base">Source search</CardTitle><CardDescription>The gateway returns ranked metadata only; selection remains explicit.</CardDescription></CardHeader>
						<CardContent class="flex flex-wrap gap-2">
							<Button onclick={() => search.mutate()} disabled={!canSearch || search.isPending}><SearchIcon data-icon="inline-start" aria-hidden="true" />{search.isPending ? 'Searching…' : 'Search sources'}</Button>
						</CardContent>
					</Card>
				{:else if canRetry}
					<Card>
						<CardContent class="flex flex-wrap items-center justify-between gap-3 pt-6"><p class="text-sm text-muted-foreground">The server marked this request retryable.</p><Button variant="outline" onclick={() => retry.mutate()} disabled={retry.isPending}><RotateCcwIcon data-icon="inline-start" aria-hidden="true" />{retry.isPending ? 'Retrying…' : `Retry (${requestRecord.maxRetries - requestRecord.retries} left)`}</Button></CardContent>
					</Card>
				{/if}
			</aside>
		</div>
	{/if}
</div>

<AlertDialog.Root open={grabConfirmOpen} onOpenChange={(open) => { if (!open && !startGrab.isPending) grabConfirmOpen = false; }}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>Start this download?</AlertDialog.Title>
			<AlertDialog.Description>
				This is the irreversible handoff to the private gateway. The server will poll download progress, verify the staged file, and queue the existing importer. No tracker URL or cookie is shown to Coppice.
				{#if selectedRelease}<strong class="mt-2 block text-foreground">{selectedRelease.title}</strong><span class="block">{selectedRelease.sourceProvider} · score {selectedRelease.score}</span>{/if}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={startGrab.isPending}>Not yet</AlertDialog.Cancel>
			<AlertDialog.Action onclick={confirmGrab} disabled={!canGrab || startGrab.isPending}>{startGrab.isPending ? 'Starting…' : 'Start download'}</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
