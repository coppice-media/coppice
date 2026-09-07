<script lang="ts">
	import { browser } from '$app/environment';
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request, subscribe } from '@stump/ui/graphql/client';
	import {
		CancelWorkerJobDocument,
		WorkerJobEventsDocument,
		WorkersDocument,
		type WorkerJobStatus
	} from '$lib/graphql/generated/graphql';

	const queryClient = useQueryClient();
	const workersQuery = createQuery(() => ({
		queryKey: ['workers'],
		queryFn: () => request(WorkersDocument, {}),
		enabled: browser
	}));

	const workers = $derived(workersQuery.data?.workers ?? []);
	const jobs = $derived(workersQuery.data?.workerJobs ?? []);
	// `needs_worker` is the whole reason the status exists: it is the work this
	// server has been asked for and cannot do. It leads.
	const needsWorker = $derived(jobs.filter((job) => job.status === 'NEEDS_WORKER'));

	let liveError = $state<string | null>(null);

	// Every transition is a `WorkerJobChanged` on the shared event socket, so
	// the list follows a running encode without polling.
	$effect(() => {
		return subscribe(
			WorkerJobEventsDocument,
			{},
			{
				next: (result) => {
					if (result.data?.readEvents?.__typename !== 'WorkerJobChanged') return;
					void queryClient.invalidateQueries({ queryKey: ['workers'] });
				},
				error: () => {
					liveError = 'Live job status is unavailable; the list updates on reload.';
				},
				complete: () => undefined
			}
		);
	});

	// `enumsAsTypes` in codegen.ts means WorkerJobStatus is a union of string
	// literals, not a value, so the statuses are written as the wire words the
	// server sends.
	function statusVariant(status: WorkerJobStatus) {
		switch (status) {
			case 'DONE':
				return 'secondary' as const;
			case 'FAILED':
				return 'destructive' as const;
			case 'NEEDS_WORKER':
				return 'outline' as const;
			default:
				return 'default' as const;
		}
	}

	function isCancellable(status: WorkerJobStatus) {
		return status !== 'DONE' && status !== 'FAILED';
	}

	async function cancel(id: string) {
		await request(CancelWorkerJobDocument, { id });
		await queryClient.invalidateQueries({ queryKey: ['workers'] });
	}
</script>

<svelte:head>
	<title>Workers · Stump</title>
</svelte:head>

<div class="flex flex-col gap-8">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight">Workers</h1>
		<p class="text-sm text-muted-foreground">
			Machines that run heavy jobs for this server. A worker pairs like any other device and
			connects out, so it never has to be reachable from here.
		</p>
	</div>

	{#if liveError}
		<Alert>
			<AlertTitle>Live updates paused</AlertTitle>
			<AlertDescription>{liveError}</AlertDescription>
		</Alert>
	{/if}

	{#if needsWorker.length > 0}
		<Alert>
			<AlertTitle>
				{needsWorker.length} job{needsWorker.length === 1 ? '' : 's'} waiting for a worker
			</AlertTitle>
			<AlertDescription>
				No connected worker advertises {[...new Set(needsWorker.map((job) => job.kind))].join(', ')},
				and this server has no built-in way to do it. Connect a worker and these start on their own.
			</AlertDescription>
		</Alert>
	{/if}

	<section class="flex flex-col gap-4">
		<h2 class="text-lg font-medium">Connected</h2>
		{#if workersQuery.isPending}
			<Skeleton class="h-24 rounded-xl" />
		{:else if workersQuery.isError}
			<Alert variant="destructive">
				<AlertTitle>Unable to load workers</AlertTitle>
				<AlertDescription>
					{workersQuery.error instanceof Error ? workersQuery.error.message : 'Request failed.'}
				</AlertDescription>
			</Alert>
		{:else if workers.length === 0}
			<Empty class="rounded-xl border border-dashed bg-card">
				<EmptyHeader>
					<EmptyTitle>No workers connected</EmptyTitle>
					<EmptyDescription>
						Add a device of kind <code>Worker</code> on the Devices page, then run
						<code>stump-worker --server &lt;this server&gt; --api-key &lt;key&gt;</code> on the machine
						that should do the work.
					</EmptyDescription>
				</EmptyHeader>
			</Empty>
		{:else}
			<div class="grid gap-4 md:grid-cols-2">
				{#each workers as worker (worker.id)}
					<div class="flex flex-col gap-2 rounded-xl border bg-card p-4">
						<div class="flex items-center gap-2">
							<span class="font-medium">{worker.name}</span>
							{#if worker.version}
								<span class="text-xs text-muted-foreground">v{worker.version}</span>
							{/if}
						</div>
						<div class="flex flex-wrap gap-1">
							{#each worker.kinds as kind (kind)}
								<Badge variant="secondary">{kind}</Badge>
							{/each}
						</div>
						<p class="text-xs text-muted-foreground">
							Connected {new Date(worker.connectedAt).toLocaleString()} · last frame
							{new Date(worker.lastSeenAt).toLocaleTimeString()}
						</p>
					</div>
				{/each}
			</div>
		{/if}
	</section>

	<section class="flex flex-col gap-4">
		<h2 class="text-lg font-medium">Jobs</h2>
		{#if workersQuery.isPending}
			<Skeleton class="h-32 rounded-xl" />
		{:else if jobs.length === 0}
			<Empty class="rounded-xl border border-dashed bg-card">
				<EmptyHeader>
					<EmptyTitle>Nothing queued</EmptyTitle>
					<EmptyDescription>
						Worker jobs appear here as soon as something asks for one — playing an audiobook on a
						device with an Opus preset is the usual first one.
					</EmptyDescription>
				</EmptyHeader>
			</Empty>
		{:else}
			<div class="overflow-hidden rounded-xl border bg-card">
				<table class="w-full text-sm">
					<thead class="bg-muted/50 text-left text-xs uppercase text-muted-foreground">
						<tr>
							<th class="px-4 py-2 font-medium">Kind</th>
							<th class="px-4 py-2 font-medium">Status</th>
							<th class="px-4 py-2 font-medium">Ran on</th>
							<th class="px-4 py-2 font-medium">Progress</th>
							<th class="px-4 py-2 font-medium">Created</th>
							<th class="px-4 py-2"></th>
						</tr>
					</thead>
					<tbody>
						{#each jobs as job (job.id)}
							<tr class="border-t">
								<td class="px-4 py-2 font-mono text-xs">{job.kind}</td>
								<td class="px-4 py-2">
									<Badge variant={statusVariant(job.status)}>{job.status}</Badge>
									{#if job.error}
										<span class="ml-2 text-xs text-muted-foreground">{job.error}</span>
									{/if}
								</td>
								<td class="px-4 py-2 text-xs text-muted-foreground">
									{#if job.workerId}
										{workers.find((worker) => worker.id === job.workerId)?.name ?? job.workerId}
									{:else}
										this server
									{/if}
								</td>
								<td class="px-4 py-2 text-xs text-muted-foreground">
									{Math.round(job.progress * 100)}%
									{#if job.message}· {job.message}{/if}
								</td>
								<td class="px-4 py-2 text-xs text-muted-foreground">
									{new Date(job.createdAt).toLocaleString()}
								</td>
								<td class="px-4 py-2 text-right">
									{#if isCancellable(job.status)}
										<Button variant="ghost" size="sm" onclick={() => cancel(job.id)}>Cancel</Button>
									{/if}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	</section>
</div>
