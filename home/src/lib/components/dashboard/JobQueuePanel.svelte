<script lang="ts">
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Progress } from '@stump/ui/components/ui/progress';
	import type { DashboardJobsQuery } from '$lib/graphql/generated/graphql';
	import {
		ACTIVE_JOB_STATUSES,
		durationLabel,
		jobLabel,
		JOB_STATUS_VARIANTS,
		type LiveJobProgress
	} from '$lib/dashboard';
	import { absoluteTime, relativeTime } from '$lib/format';
	import DashboardSection from './DashboardSection.svelte';

	type Job = DashboardJobsQuery['jobs']['nodes'][number];

	// `jobs` is the persisted history (newest first); `live` is what the
	// `readEvents` stream has said about those ids since the page loaded, so a
	// running job shows its current task without refetching on every tick.
	let {
		jobs,
		live,
		queued,
		queuedByKind,
		pending = false,
		error = null,
		liveError = null,
		now = new Date()
	}: {
		jobs: readonly Job[];
		live: Record<string, LiveJobProgress>;
		queued: number | null;
		queuedByKind: Record<string, number>;
		pending?: boolean;
		error?: unknown;
		liveError?: string | null;
		now?: Date;
	} = $props();

	const running = $derived(
		jobs.filter((job) => ACTIVE_JOB_STATUSES.includes(live[job.id]?.status ?? job.status))
	);
	const queueLabel = $derived.by(() => {
		const count = queued ?? running.length;
		const kinds = Object.entries(queuedByKind)
			.filter(([, value]) => value > 0)
			.map(([kind, value]) => `${value} ${jobLabel(kind).toLowerCase()}`);
		if (count === 0) {
			return jobs.length
				? `Nothing queued · the ${jobs.length} most recent runs`
				: 'Nothing queued.';
		}
		const head = count === 1 ? '1 job in the queue' : `${count} jobs in the queue`;
		return kinds.length ? `${head} · ${kinds.join(', ')}` : head;
	});
</script>

<DashboardSection
	title="Job queue"
	description={queueLabel}
	{pending}
	{error}
	errorTitle="Unable to load jobs"
	emptyTitle="No jobs yet"
	emptyDescription="Library scans, thumbnail generation, and provider probes appear here."
	empty={jobs.length === 0}
>
	{#snippet action()}
		{#if liveError}
			<Badge variant="outline" title={liveError}>Live updates paused</Badge>
		{:else}
			<Badge variant="secondary">Live</Badge>
		{/if}
	{/snippet}
	<ul class="flex flex-col divide-y">
		{#each jobs as job (job.id)}
			{@const update = live[job.id]}
			{@const status = update?.status ?? job.status}
			{@const total = update?.remainingTasks ?? 0}
			{@const done = update?.completedTasks ?? 0}
			<li class="flex flex-col gap-1 py-2 first:pt-0 last:pb-0">
				<div class="flex flex-wrap items-baseline gap-2">
					<span class="text-sm font-medium">{jobLabel(job.name)}</span>
					<Badge variant={JOB_STATUS_VARIANTS[status]}>{status.toLowerCase()}</Badge>
					<span class="ml-auto text-xs text-muted-foreground" title={absoluteTime(job.createdAt)}>
						{relativeTime(job.completedAt ?? job.createdAt, now)}
						{#if job.completedAt}· {durationLabel(job.msElapsed)}{/if}
					</span>
				</div>
				<span class="text-xs text-muted-foreground">
					{update?.message ?? job.description ?? 'No detail reported.'}
					{#if update?.subtitle}· {update.subtitle}{/if}
				</span>
				{#if total > 0 && status === 'RUNNING'}
					<Progress value={done} max={total} class="mt-1" />
					<span class="text-[10px] tabular-nums text-muted-foreground">{done} / {total} tasks</span>
				{/if}
			</li>
		{/each}
	</ul>
</DashboardSection>
