<script lang="ts">
	import { browser } from '$app/environment'
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query'
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert'
	import HistoryIcon from '@lucide/svelte/icons/history'
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw'
	import XCircleIcon from '@lucide/svelte/icons/x-circle'
	import { toast } from 'svelte-sonner'
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert'
	import { Badge } from '@stump/ui/components/ui/badge'
	import { Button } from '@stump/ui/components/ui/button'
	import * as Card from '@stump/ui/components/ui/card'
	import { Skeleton } from '@stump/ui/components/ui/skeleton'
	import { request } from '@stump/ui/graphql/client'
	import { errorMessage } from '@stump/ui/utils/errors.js'
	import {
		CancelCrosspointDeliveryDocument,
		CrosspointDeliveriesDocument,
		RetryCrosspointDeliveryDocument
	} from '$lib/graphql/generated/graphql'
	import type { CrosspointTargetFieldsFragment } from '$lib/graphql/generated/graphql'

	import { absoluteTime, bytesLabel, relativeTime } from '$lib/format'

	type CrosspointTransferProfile = CrosspointTargetFieldsFragment['profile']

	let { deviceId }: { deviceId: string } = $props()

	const queryClient = useQueryClient()
	const deliveriesQuery = createQuery(() => ({
		queryKey: ['crosspoint-deliveries', deviceId],
		queryFn: () => request(CrosspointDeliveriesDocument, { deviceId, limit: 50 }),
		enabled: browser && !!deviceId,
		refetchInterval: 3_000
	}))
	const deliveries = $derived(deliveriesQuery.data?.crosspointDeliveries ?? [])

	const retry = createMutation(() => ({
		mutationFn: (id: string) => request(RetryCrosspointDeliveryDocument, { id }),
		onSuccess: () => {
			toast.success('Delivery returned to the local queue.')
			void queryClient.invalidateQueries({ queryKey: ['crosspoint-deliveries', deviceId] })
		},
		onError: (error) => toast.error(errorMessage(error))
	}))
	const cancel = createMutation(() => ({
		mutationFn: (id: string) => request(CancelCrosspointDeliveryDocument, { id }),
		onSuccess: () => {
			toast.success('Local delivery cancelled.')
			void queryClient.invalidateQueries({ queryKey: ['crosspoint-deliveries', deviceId] })
		},
		onError: (error) => toast.error(errorMessage(error))
	}))

	type SourceRevision = {
		path: string
		hash: string
		filename: string
		bytes: number
	}

	function sourceRevisionOf(value: unknown): SourceRevision {
		if (typeof value === 'string') {
			try {
				const parsed: unknown = JSON.parse(value)
				return typeof parsed === 'string' ? sourceRevisionOf({ path: parsed }) : sourceRevisionOf(parsed)
			} catch {
				return { path: value, hash: '—', filename: value.split('/').pop() || 'book', bytes: 0 }
			}
		}
		if (!value || typeof value !== 'object') {
			return { path: '—', hash: '—', filename: 'book', bytes: 0 }
		}
		const record = value as Record<string, unknown>
		return {
			path: typeof record.path === 'string' ? record.path : '—',
			hash: typeof record.hash === 'string' ? record.hash : '—',
			filename: typeof record.filename === 'string' ? record.filename : 'book',
			bytes: typeof record.bytes === 'number' ? record.bytes : 0
		}
	}
	function profileSnapshot(value: unknown): CrosspointTransferProfile | null {
		let candidate = value
		if (typeof candidate === 'string') {
			try {
				candidate = JSON.parse(candidate)
			} catch {
				return null
			}
		}
		if (!candidate || typeof candidate !== 'object') return null
		const record = candidate as Record<string, unknown>
		const model = record.targetModel
		const booleans = ['optimizerEnabled', 'grayscale', 'autoCrop', 'splitLargeParagraphs', 'removeFonts']
		const numbers = ['jpegQuality', 'chunkBytes', 'retryCount', 'retryDelaySeconds', 'timeoutSeconds', 'maxUploadBytes']
		if (model !== 'AUTO' && model !== 'X3' && model !== 'X4') return null
		if (booleans.some((key) => typeof record[key] !== 'boolean') || numbers.some((key) => typeof record[key] !== 'number')) return null
		return record as CrosspointTransferProfile
	}

	function profileLabel(value: unknown): string {
		const profile = profileSnapshot(value)
		if (!profile) return 'snapshot unavailable'
		return `${profile.targetModel} · JPEG ${profile.jpegQuality} · ${profile.grayscale ? 'grayscale' : 'color'} · ${profile.chunkBytes} B chunks · ${profile.retryCount} retries (${profile.retryCount + 1} attempts)`
	}

	function canRetry(status: string): boolean {
		return status === 'FAILED'
	}

	function canCancel(status: string): boolean {
		return status === 'QUEUED' || status === 'PREPARING'
	}

	function statusLabel(status: string): string {
		switch (status) {
			case 'QUEUED':
				return 'Queued'
			case 'PREPARING':
				return 'Preparing'
			case 'TRANSFERRING':
				return 'Transferring'
			case 'COMPLETED':
				return 'Succeeded'
			case 'FAILED':
				return 'Failed'
			case 'CANCELLED':
				return 'Cancelled'
			default:
				return status
		}
	}

	function statusVariant(status: string): 'default' | 'secondary' | 'outline' | 'destructive' {
		if (status === 'FAILED') return 'destructive'
		if (status === 'COMPLETED') return 'default'
		if (status === 'CANCELLED') return 'outline'
		return 'secondary'
	}
</script>

<Card.Root size="sm" class="gap-0">
	<Card.Header class="gap-2">
		<div class="flex items-start gap-3">
			<span class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
				<HistoryIcon class="size-4" aria-hidden="true" />
			</span>
			<div class="min-w-0">
				<Card.Title class="text-base">Delivery queue and history</Card.Title>
				<Card.Description>Local CrossPoint work for this device, newest first.</Card.Description>
			</div>
		</div>
	</Card.Header>
	<Card.Content class="flex flex-col gap-3 pt-0">
		<p class="rounded-lg border border-dashed px-3 py-2 text-xs text-muted-foreground">
			Queue items keep source revision, optimizer profile, destination path, and idempotency snapshots. Coppice never overwrites or deletes a remote file; a failed item can be retried locally after fixing the device or target.
		</p>

		{#if deliveriesQuery.isPending}
			<div class="flex flex-col gap-2" aria-label="Loading CrossPoint delivery history">
				<Skeleton class="h-24 rounded-lg" />
				<Skeleton class="h-24 rounded-lg" />
			</div>
		{:else if deliveriesQuery.isError}
			<Alert variant="destructive">
				<CircleAlertIcon data-icon="inline-start" aria-hidden="true" />
				<AlertTitle>Unable to load delivery history</AlertTitle>
				<AlertDescription>{errorMessage(deliveriesQuery.error)}</AlertDescription>
				<Button type="button" variant="outline" size="sm" class="mt-2 w-fit" onclick={() => deliveriesQuery.refetch()}>
					Retry
				</Button>
			</Alert>
		{:else if deliveries.length === 0}
			<div class="rounded-lg border border-dashed bg-muted/20 px-3 py-5 text-center">
				<p class="text-sm font-medium">No CrossPoint deliveries yet</p>
				<p class="mt-1 text-xs text-muted-foreground">Use Send to CrossPoint on a visible book. Queueing waits for backend confirmation; it does not claim physical completion.</p>
			</div>
		{:else}
			<div class="flex flex-col gap-2" aria-label="CrossPoint delivery history">
				{#each deliveries as delivery (delivery.id)}
					{@const source = sourceRevisionOf(delivery.sourceRevision)}
					{@const typedProfile = profileSnapshot(delivery.profileJson)}
					{@const busy = retry.isPending || cancel.isPending}
					<div class="flex flex-col gap-3 rounded-lg border bg-muted/10 p-3">
						<div class="flex flex-wrap items-start gap-2">
							<div class="min-w-0 flex-1">
								<p class="truncate text-sm font-medium" title={source.path}>{source.filename}</p>
								<p class="text-xs text-muted-foreground">{delivery.mediaId} · {bytesLabel(source.bytes)} · {delivery.attempts}/{delivery.maxAttempts} attempts</p>
							</div>
							<Badge variant={statusVariant(String(delivery.status))}>{statusLabel(String(delivery.status))}</Badge>
						</div>
						<dl class="grid gap-x-4 gap-y-1 text-xs sm:grid-cols-2">
							<div class="min-w-0"><dt class="text-muted-foreground">Source revision</dt><dd class="truncate font-mono" title={source.path}>{source.path}</dd></div>
							<div class="min-w-0"><dt class="text-muted-foreground">Source hash</dt><dd class="truncate font-mono" title={source.hash}>{source.hash}</dd></div>
							<div class="min-w-0"><dt class="text-muted-foreground">Profile snapshot</dt><dd class="truncate" title={String(delivery.profileJson)}><span class="font-mono">{delivery.profileDigest}</span> · {typedProfile ? profileLabel(typedProfile) : 'snapshot unavailable'}</dd></div>
							<div class="min-w-0"><dt class="text-muted-foreground">Destination snapshot</dt><dd class="truncate font-mono" title={`${delivery.destinationPath}/${source.filename}`}>{delivery.destinationPath}/{source.filename}</dd></div>
						</dl>
						<div class="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
							<span title={absoluteTime(delivery.queuedAt)}>Queued {relativeTime(delivery.queuedAt)}</span>
							{#if delivery.startedAt}<span title={absoluteTime(delivery.startedAt)}>Started {relativeTime(delivery.startedAt)}</span>{/if}
							{#if delivery.completedAt}<span title={absoluteTime(delivery.completedAt)}>Finished {relativeTime(delivery.completedAt)}</span>{/if}
							{#if delivery.nextAttemptAt}<span title={absoluteTime(delivery.nextAttemptAt)}>Next attempt {relativeTime(delivery.nextAttemptAt)}</span>{/if}
						</div>
						{#if delivery.lastError}
							<p class="flex items-start gap-1.5 rounded-md border border-destructive/30 bg-destructive/5 px-2 py-1.5 text-xs text-destructive" role="alert">
								<CircleAlertIcon class="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
								<span><strong class="font-medium">Action needed:</strong> {delivery.lastError}</span>
							</p>
						{/if}
						{#if canRetry(String(delivery.status)) || canCancel(String(delivery.status))}
							<div class="flex flex-wrap gap-1.5 border-t pt-2">
								{#if canRetry(String(delivery.status))}
									<Button type="button" size="xs" variant="outline" onclick={() => retry.mutate(delivery.id)} disabled={busy}>
										<RefreshCwIcon data-icon="inline-start" aria-hidden="true" />
										{retry.isPending ? 'Retrying…' : 'Retry'}
									</Button>
								{/if}
								{#if canCancel(String(delivery.status))}
									<Button type="button" size="xs" variant="ghost" onclick={() => cancel.mutate(delivery.id)} disabled={busy}>
										<XCircleIcon data-icon="inline-start" aria-hidden="true" />
										{cancel.isPending ? 'Cancelling…' : 'Cancel local work'}
									</Button>
								{/if}
							</div>
						{/if}
					</div>
				{/each}
			</div>
		{/if}
	</Card.Content>
</Card.Root>
