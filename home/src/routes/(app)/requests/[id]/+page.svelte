<script lang="ts">
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import XCircleIcon from '@lucide/svelte/icons/x-circle';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import {
		ApproveBookRequestDocument,
		BookRequestDocument,
		RejectBookRequestDocument,
		RequestDestinationsDocument,
		SetBookRequestPreferredNarratorDocument
	} from '$lib/graphql/generated/graphql';
	import { getHomeSession } from '$lib/session.svelte';
	import { absoluteTime, relativeTime } from '$lib/format';
	import { canEditNarrator, requestFormatLabel, safeCoverUrl, statusDescription } from '$lib/requests';
	import RequestAcquisitionPanel from '$lib/components/requests/RequestAcquisitionPanel.svelte';
	import RequestFormatControl from '$lib/components/requests/RequestFormatControl.svelte';
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

	const canAcquire = $derived(
		Boolean(
			session.user?.isServerOwner ||
				session.user?.permissions.some((permission) => String(permission) === 'ACQUIRE_RELEASES')
		)
	);

	const detailQuery = createQuery(() => ({
		queryKey: ['book-request', requestId],
		queryFn: () => request(BookRequestDocument, { id: requestId }),
		enabled: browser && !!requestId
	}));
	const requestRecord = $derived(detailQuery.data?.bookRequest ?? null);
	const cover = $derived(safeCoverUrl(requestRecord?.coverUrl));
	const requestStatus = $derived(requestRecord?.status ?? '');
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

	function invalidate(): void {
		void queryClient.invalidateQueries({ queryKey: ['book-request', requestId] });
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
		onError: (error) => showError(error, 'Rejection failed.')
	}));

	const canDecide = $derived(canManage && ['PENDING', 'AWAITING_APPROVAL'].includes(requestStatus));

	// The requester or a request manager may re-aim the narrator preference
	// until the request is fulfilled or rejected; the server enforces the same.
	const narratorEditable = $derived(
		Boolean(requestRecord) &&
			(canManage || requestRecord?.requesterId === session.user?.id) &&
			canEditNarrator(requestRecord?.format, requestStatus)
	);
	const setNarrator = createMutation(() => ({
		mutationFn: (narrator: string | null) =>
			request(SetBookRequestPreferredNarratorDocument, { requestId, narrator }),
		onSuccess: () => {
			actionError = null;
			invalidate();
		},
		onError: (error) => showError(error, 'The narrator preference could not be saved.')
	}));
	const narratorLookup = $derived(
		requestRecord?.sourceProvider && requestRecord.remoteId
			? {
					provider: requestRecord.sourceProvider,
					remoteId: requestRecord.remoteId,
					title: requestRecord.title,
					authors: requestRecord.authors
				}
			: null
	);
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
					{#if cover}
						<img src={cover} alt="" class="size-16 rounded-md border object-cover" />
					{/if}
					<div>
						<div class="flex flex-wrap items-center gap-2">
							<h1 class="text-2xl font-semibold tracking-tight">{requestRecord.title}</h1>
							<RequestStatusBadge status={requestRecord.status} />
							{#if requestRecord.preferredNarrator}
								<Badge variant="outline">Narrator: {requestRecord.preferredNarrator}</Badge>
							{/if}
						</div>
						<p class="mt-1 text-sm text-muted-foreground">
							{requestRecord.authors || 'Author not provided'}{requestRecord.sourceProvider ? ` · ${requestRecord.sourceProvider}` : ''}
						</p>
					</div>
				</div>
			{/if}
		</div>
		<Button variant="outline" href={resolve('/requests/new')}>New request</Button>
	</div>

	{#if detailQuery.isPending}
		<div class="grid gap-5 xl:grid-cols-[minmax(0,1fr)_22rem]">
			<Skeleton class="h-96 rounded-xl" />
			<Skeleton class="h-96 rounded-xl" />
		</div>
	{:else if detailQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load request</AlertTitle>
			<AlertDescription>
				{detailQuery.error instanceof Error ? detailQuery.error.message : 'The request could not be loaded or is not visible to this account.'}
			</AlertDescription>
		</Alert>
	{:else if requestRecord}
		{#if actionError}
			<Alert variant="destructive">
				<AlertTitle>Request action failed</AlertTitle>
				<AlertDescription>{actionError}</AlertDescription>
			</Alert>
		{/if}

		<Card>
			<CardHeader class="flex flex-row items-start gap-3 space-y-0">
				<div class="mr-auto">
					<CardTitle class="text-base">Request status</CardTitle>
					<CardDescription class="mt-1">{statusDescription(requestRecord.status)}</CardDescription>
				</div>
				<RequestStatusBadge status={requestRecord.status} />
			</CardHeader>
			{#if requestRecord.failureMessage || requestRecord.failureCode}
				<CardContent class="pt-0">
					<p class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
						{requestRecord.failureMessage || requestRecord.failureCode}
					</p>
				</CardContent>
			{/if}
		</Card>
		{#if canAcquire && requestRecord.status === 'APPROVED'}
			<RequestAcquisitionPanel
				requestId={requestRecord.id}
				requestTitle={requestRecord.title}
				format={requestRecord.format}
				isbn={requestRecord.isbn}
			/>
		{/if}

		<div class="grid gap-5 xl:grid-cols-[minmax(0,1fr)_22rem]">
			<div class="flex min-w-0 flex-col gap-5">
				<Card>
					<CardHeader>
						<CardTitle class="text-base">Requested work</CardTitle>
						<CardDescription>Metadata recorded when this request was submitted.</CardDescription>
					</CardHeader>
					<CardContent>
						<dl class="grid gap-x-5 gap-y-4 text-sm sm:grid-cols-2">
							<div>
								<dt class="text-muted-foreground">Title</dt>
								<dd class="mt-1 font-medium">{requestRecord.title}</dd>
							</div>
							<div>
								<dt class="text-muted-foreground">Authors</dt>
								<dd class="mt-1 font-medium">{requestRecord.authors || 'Not provided'}</dd>
							</div>
							<div>
								<dt class="text-muted-foreground">Format</dt>
								<dd class="mt-1 font-medium">{requestFormatLabel(requestRecord.format)}</dd>
							</div>
							<div>
								<dt class="text-muted-foreground">Narrator</dt>
								<dd class="mt-1">
									{#if narratorEditable}
										<RequestFormatControl
											value={requestRecord.format}
											narrator={requestRecord.preferredNarrator ?? null}
											lookup={narratorLookup}
											lockFormat
											disabled={setNarrator.isPending}
											label="Requested format"
											onnarratorchange={(narrator) => setNarrator.mutate(narrator)}
										/>
									{:else}
										<span class="font-medium">{requestRecord.preferredNarrator || 'Any narrator'}</span>
									{/if}
								</dd>
							</div>
							<div>
								<dt class="text-muted-foreground">ISBN</dt>
								<dd class="mt-1 font-medium">{requestRecord.isbn || 'Not provided'}</dd>
							</div>
							{#if requestRecord.sourceProvider}
								<div>
									<dt class="text-muted-foreground">Metadata provider</dt>
									<dd class="mt-1 font-medium">{requestRecord.sourceProvider}</dd>
								</div>
							{/if}
							{#if requestRecord.remoteId}
								<div>
									<dt class="text-muted-foreground">Provider reference</dt>
									<dd class="mt-1 break-all font-mono text-xs">{requestRecord.remoteId}</dd>
								</div>
							{/if}
							{#if requestRecord.externalKey}
								<div>
									<dt class="text-muted-foreground">External key</dt>
									<dd class="mt-1 break-all font-mono text-xs">{requestRecord.externalKey}</dd>
								</div>
							{/if}
							{#if requestRecord.internalMediaId || requestRecord.internalWorkId}
								<div>
									<dt class="text-muted-foreground">Library reference</dt>
									<dd class="mt-1 break-all font-mono text-xs">{requestRecord.internalMediaId || requestRecord.internalWorkId}</dd>
								</div>
							{/if}
						</dl>
					</CardContent>
				</Card>

				{#if canDecide}
					<Card>
						<CardHeader>
							<CardTitle class="text-base">Review request</CardTitle>
							<CardDescription>Record an approval or rejection in Coppice. Approval does not start acquisition.</CardDescription>
						</CardHeader>
						<CardContent class="flex flex-col gap-4">
							<div class="flex flex-col gap-2">
								<Label for="request-decision-reason">Reason <span class="font-normal text-muted-foreground">(optional)</span></Label>
								<Input id="request-decision-reason" bind:value={decisionReason} placeholder="Decision note" />
							</div>
							<div class="flex flex-wrap gap-2">
								<Button onclick={() => approve.mutate()} disabled={approve.isPending || reject.isPending}>
									<ShieldCheckIcon data-icon="inline-start" aria-hidden="true" />
									{approve.isPending ? 'Approving…' : 'Approve'}
								</Button>
								<Button variant="destructive" onclick={() => reject.mutate()} disabled={approve.isPending || reject.isPending}>
									<XCircleIcon data-icon="inline-start" aria-hidden="true" />
									{reject.isPending ? 'Rejecting…' : 'Reject'}
								</Button>
							</div>
						</CardContent>
					</Card>
				{/if}
			</div>

			<div class="flex flex-col gap-5">
				<Card>
					<CardHeader>
						<CardTitle class="text-base">Requested destination</CardTitle>
						<CardDescription>Optional intent saved with the ledger entry.</CardDescription>
					</CardHeader>
					<CardContent>
						<dl class="grid gap-3 text-sm">
							<div>
								<dt class="text-muted-foreground">Shelf</dt>
								<dd class="mt-1 font-medium">{shelfName || 'No shelf selected'}</dd>
							</div>
							<div>
								<dt class="text-muted-foreground">Device</dt>
								<dd class="mt-1 font-medium">{deviceName || 'No device selected'}</dd>
							</div>
						</dl>
					</CardContent>
				</Card>

				<Card>
					<CardHeader>
						<CardTitle class="text-base">Ledger</CardTitle>
					</CardHeader>
					<CardContent>
						<dl class="grid gap-3 text-sm">
							<div>
								<dt class="text-muted-foreground">Requester</dt>
								<dd class="mt-1 break-all font-mono text-xs">{requestRecord.requesterId}</dd>
							</div>
							<div>
								<dt class="text-muted-foreground">Approval policy</dt>
								<dd class="mt-1 font-medium">{requestRecord.approvalPolicy.replace(/_/g, ' ').toLowerCase()}</dd>
							</div>
							<div>
								<dt class="text-muted-foreground">Created</dt>
								<dd class="mt-1 font-medium" title={absoluteTime(requestRecord.createdAt)}>{relativeTime(requestRecord.createdAt)}</dd>
							</div>
							<div>
								<dt class="text-muted-foreground">Updated</dt>
								<dd class="mt-1 font-medium" title={absoluteTime(requestRecord.updatedAt)}>{relativeTime(requestRecord.updatedAt)}</dd>
							</div>
							{#if requestRecord.approvedAt}
								<div>
									<dt class="text-muted-foreground">Approved</dt>
									<dd class="mt-1 font-medium" title={absoluteTime(requestRecord.approvedAt)}>{relativeTime(requestRecord.approvedAt)}</dd>
								</div>
							{/if}
							{#if requestRecord.approvedBy}
								<div>
									<dt class="text-muted-foreground">Approved by</dt>
									<dd class="mt-1 break-all font-mono text-xs">{requestRecord.approvedBy}</dd>
								</div>
							{/if}
							{#if requestRecord.rejectedBy}
								<div>
									<dt class="text-muted-foreground">Rejected by</dt>
									<dd class="mt-1 break-all font-mono text-xs">{requestRecord.rejectedBy}</dd>
								</div>
							{/if}
							{#if requestRecord.completedAt}
								<div>
									<dt class="text-muted-foreground">Historical completion</dt>
									<dd class="mt-1 font-medium" title={absoluteTime(requestRecord.completedAt)}>{relativeTime(requestRecord.completedAt)}</dd>
								</div>
							{/if}
						</dl>
					</CardContent>
				</Card>
			</div>
		</div>
	{/if}
</div>
