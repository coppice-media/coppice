<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Separator } from '@stump/ui/components/ui/separator';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@stump/ui/components/ui/table';
	import ProgressIndicator from '$lib/components/ProgressIndicator.svelte';
	import StatusBadge from '$lib/components/StatusBadge.svelte';
	import { request } from '@stump/ui/graphql/client';
	import {
		CancelIngestAnalysisDocument,
		IngestAnalysisQueueDocument,
		PauseIngestAnalysisDocument,
		RetryIngestAnalysisDocument,
		ResumeIngestAnalysisDocument,
		type JobStatus
	} from '$lib/graphql/generated/graphql';
	import { humanize, offsetPagination } from '$lib/ingest/helpers';

	const queryClient = useQueryClient();
	let statusFilter = $state<JobStatus | ''>('');
	const queueQuery = createQuery(() => ({
		queryKey: ['analysis-queue', statusFilter],
		queryFn: () =>
			request(IngestAnalysisQueueDocument, {
				status: statusFilter || null,
				pagination: offsetPagination(100)
			}),
		enabled: browser
	}));

	function refreshQueue(): void {
		void queryClient.invalidateQueries({ queryKey: ['analysis-queue'] });
		void queryClient.invalidateQueries({ queryKey: ['drop-items'] });
	}

	const pauseMutation = createMutation(() => ({
		mutationFn: (jobId: string) => request(PauseIngestAnalysisDocument, { jobId }),
		onSuccess: () => { refreshQueue(); toast.success('Analysis paused.'); },
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to pause analysis.')
	}));
	const resumeMutation = createMutation(() => ({
		mutationFn: (jobId: string) => request(ResumeIngestAnalysisDocument, { jobId }),
		onSuccess: () => { refreshQueue(); toast.success('Analysis resumed.'); },
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to resume analysis.')
	}));
	const retryMutation = createMutation(() => ({
		mutationFn: (jobId: string) => request(RetryIngestAnalysisDocument, { jobId }),
		onSuccess: () => { refreshQueue(); toast.success('Analysis retried.'); },
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to retry analysis.')
	}));
	const cancelMutation = createMutation(() => ({
		mutationFn: (jobId: string) => request(CancelIngestAnalysisDocument, { jobId }),
		onSuccess: () => { refreshQueue(); toast.success('Analysis cancelled.'); },
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to cancel analysis.')
	}));

	let jobs = $derived(queueQuery.data?.ingestAnalysisQueue.nodes ?? []);
	let totalJobs = $derived(
		queueQuery.data?.ingestAnalysisQueue.pageInfo && 'totalItems' in queueQuery.data.ingestAnalysisQueue.pageInfo
			? queueQuery.data.ingestAnalysisQueue.pageInfo.totalItems
			: jobs.length
	);

	function isBusy(): boolean {
		return pauseMutation.isPending || resumeMutation.isPending || retryMutation.isPending || cancelMutation.isPending;
	}
</script>

<svelte:head><title>Analysis queue · Coppice ingest</title></svelte:head>

<div class="flex flex-col gap-6">
	<div class="flex flex-wrap items-end justify-between gap-4">
		<div>
			<p class="text-sm font-medium text-primary">Orchestration</p>
			<h1 class="text-3xl font-semibold tracking-tight">Analysis queue</h1>
			<p class="mt-1 max-w-2xl text-muted-foreground">Quality-first priority keeps the lowest-scoring work visible while every phase remains observable.</p>
		</div>
		<label class="flex items-center gap-2 text-sm"><span class="text-muted-foreground">Status</span><select class="h-9 rounded-md border bg-background px-3" value={statusFilter} onchange={(event) => (statusFilter = (event.currentTarget as HTMLSelectElement).value as JobStatus | '')}><option value="">All statuses</option><option value="QUEUED">Queued</option><option value="RUNNING">Running</option><option value="PAUSED">Paused</option><option value="COMPLETED">Completed</option><option value="CANCELLED">Cancelled</option><option value="FAILED">Failed</option></select></label>
	</div>
	<Card>
		<CardHeader>
			<CardTitle>Durable jobs</CardTitle>
			<CardDescription>{totalJobs} job{totalJobs === 1 ? '' : 's'} in the editor projection</CardDescription>
		</CardHeader>
		<CardContent class="p-0">
			{#if queueQuery.isPending}
				<div class="flex flex-col gap-3 p-6">{#each Array(5) as _, index (index)}<Skeleton class="h-12 w-full" />{/each}</div>
			{:else if queueQuery.isError}
				<div class="p-6"><Alert variant="destructive"><AlertTitle>Unable to load analysis queue</AlertTitle><AlertDescription>{queueQuery.error instanceof Error ? queueQuery.error.message : 'The server did not return queue jobs.'}</AlertDescription></Alert></div>
			{:else if !jobs.length}
				<div class="p-6"><Empty><EmptyHeader><EmptyTitle>Queue is empty</EmptyTitle><EmptyDescription>Enqueue staged items from the drop folder to start analysis.</EmptyDescription></EmptyHeader></Empty></div>
			{:else}
				<div class="overflow-x-auto">
					<Table>
						<TableHeader><TableRow><TableHead>Priority</TableHead><TableHead>Phase</TableHead><TableHead>Status</TableHead><TableHead>Progress</TableHead><TableHead>Attempt</TableHead><TableHead class="text-right">Controls</TableHead></TableRow></TableHeader>
						<TableBody>
							{#each jobs as job (job.id)}
								<TableRow>
									<TableCell><div class="font-medium tabular-nums">{job.priorityScore.toFixed(0)}</div><div class="text-xs text-muted-foreground">{job.id.slice(0, 10)}</div></TableCell>
									<TableCell class="whitespace-nowrap">{humanize(job.phase)}</TableCell>
									<TableCell><StatusBadge status={job.status} /></TableCell>
									<TableCell class="min-w-44"><ProgressIndicator analysisJobId={job.id} compact /></TableCell>
									<TableCell class="tabular-nums text-muted-foreground">{job.attempt}</TableCell>
									<TableCell><div class="flex justify-end gap-2">{#if job.status === 'QUEUED' || job.status === 'RUNNING'}<Button size="sm" variant="outline" disabled={isBusy()} onclick={() => pauseMutation.mutate(job.id)}>Pause</Button>{/if}{#if job.status === 'PAUSED'}<Button size="sm" variant="outline" disabled={isBusy()} onclick={() => resumeMutation.mutate(job.id)}>Resume</Button>{/if}{#if job.status === 'FAILED' || job.status === 'CANCELLED'}<Button size="sm" variant="outline" disabled={isBusy()} onclick={() => retryMutation.mutate(job.id)}>Retry</Button>{/if}{#if job.status === 'QUEUED' || job.status === 'RUNNING' || job.status === 'PAUSED'}<Button size="sm" variant="ghost" disabled={isBusy()} onclick={() => cancelMutation.mutate(job.id)}>Cancel</Button>{/if}</div></TableCell>
								</TableRow>
							{/each}
						</TableBody>
					</Table>
				</div>
			{/if}
		</CardContent>
	</Card>
	<Separator />
	<p class="text-sm text-muted-foreground">Live phase updates come from the retained <code>ingestProgress</code> stream; reconnecting after an expired cursor refetches queue and item state.</p>
</div>
