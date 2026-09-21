<script lang="ts">
	/**
	 * The server's job queue, for users who may read it. Collapsed by default:
	 * the summary row already says whether anything is running, and a reader's
	 * dashboard is about reading, not scans.
	 */
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Card, CardContent } from '@stump/ui/components/ui/card';
	import { Progress } from '@stump/ui/components/ui/progress';
	import { QueryState } from '@stump/ui/components/ui/query-state';
	import type { DashboardJobsQuery } from '$lib/graphql/generated/graphql';
	import {
		ACTIVE_JOB_STATUSES,
		durationLabel,
		jobLabel,
		JOB_STATUS_VARIANTS,
		type LiveJobProgress
	} from '$lib/dashboard';
	import { absoluteTime, relativeTime } from '$lib/format';

	type Job = DashboardJobsQuery['jobs']['nodes'][number];

	// `jobs` is the persisted history (newest first); `live` is what the
	// `readEvents` stream has said about those ids since the page loaded, so a
	// running job shows its current task without refetching on every tick.
	let {
		jobs,
		live,
		queued,
		queuedByKind,
		query,
		liveError = null,
		now = new Date(),
		class: className
	}: {
		jobs: readonly Job[];
		live: Record<string, LiveJobProgress>;
		queued: number | null;
		queuedByKind: Record<string, number>;
		query: { isPending: boolean; error: unknown; refetch: () => unknown };
		liveError?: string | null;
		now?: Date;
		class?: string;
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
			return jobs.length ? `Idle · the ${jobs.length} most recent runs` : 'Idle';
		}
		const head = count === 1 ? '1 job in the queue' : `${count} jobs in the queue`;
		return kinds.length ? `${head} · ${kinds.join(', ')}` : head;
	});
</script>

<Card class={className}>
	<details class="group/jobs -my-(--card-spacing)">
		<summary
			class="flex cursor-pointer list-none items-center gap-3 px-(--card-spacing) py-4 outline-none select-none focus-visible:ring-3 focus-visible:ring-ring/50 [&::-webkit-details-marker]:hidden"
		>
			<ChevronDownIcon
				class="size-4 shrink-0 text-muted-foreground transition-transform group-open/jobs:rotate-180"
				aria-hidden="true"
			/>
			<span class="text-base font-medium">Job queue</span>
			<span class="min-w-0 truncate text-sm text-muted-foreground">{queueLabel}</span>
			<span class="ml-auto shrink-0">
				{#if liveError}
					<Badge variant="outline" title={liveError}>Live updates paused</Badge>
				{:else if running.length > 0 || (queued ?? 0) > 0}
					<Badge>Running</Badge>
				{:else}
					<Badge variant="secondary">Live</Badge>
				{/if}
			</span>
		</summary>
		<CardContent class="pb-(--card-spacing)">
			<QueryState
				{query}
				empty={jobs.length === 0}
				emptyTitle="No jobs yet"
				emptyDescription="Library scans, thumbnail generation, and provider probes appear here."
				errorTitle="Unable to load jobs"
				rows={3}
			>
				<ul class="flex flex-col divide-y">
					{#each jobs as job (job.id)}
						{@const update = live[job.id]}
						{@const status = update?.status ?? job.status}
						{@const total = update?.remainingTasks ?? 0}
						{@const done = update?.completedTasks ?? 0}
						<li class="flex flex-col gap-1 py-3 first:pt-0 last:pb-0">
							<div class="flex flex-wrap items-baseline gap-2">
								<span class="text-sm font-medium">{jobLabel(job.name)}</span>
								<Badge variant={JOB_STATUS_VARIANTS[status]}>{status.toLowerCase()}</Badge>
								<span
									class="ml-auto text-xs text-muted-foreground"
									title={absoluteTime(job.createdAt)}
								>
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
								<span class="text-[10px] tabular-nums text-muted-foreground">
									{done} / {total} tasks
								</span>
							{/if}
						</li>
					{/each}
				</ul>
			</QueryState>
		</CardContent>
	</details>
</Card>
