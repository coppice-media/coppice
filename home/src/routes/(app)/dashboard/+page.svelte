<script lang="ts">
	/**
	 * `/dashboard` — the landing screen, as a grid of independent widgets.
	 *
	 * Every widget owns one query and its own loading / error / empty state,
	 * so the first paint is a grid of skeletons that resolve one by one and
	 * an empty shelf never blanks the tile beside it.
	 */
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import BellIcon from '@lucide/svelte/icons/bell';
	import { Button } from '@stump/ui/components/ui/button';
	import { PageHeader } from '@stump/ui/components/ui/page-header';
	import { toast } from 'svelte-sonner';
	import { request, subscribe } from '@stump/ui/graphql/client';
	import {
		DashboardJobsDocument,
		DashboardKeepReadingDocument,
		DashboardLiveEventsDocument,
		DashboardRecentAnnotationsDocument,
		DashboardRecentlyAddedDocument,
		DashboardViewerDocument,
		DevicesDocument,
		ReadingStatsDocument,
		type ReadingStatsSpan
	} from '$lib/graphql/generated/graphql';
	import type { LiveJobProgress } from '$lib/dashboard';
	import { getHomeSession } from '$lib/session.svelte';
	import ContinueReadingShelf from '$lib/components/dashboard/ContinueReadingShelf.svelte';
	import DevicesWidget from '$lib/components/dashboard/DevicesWidget.svelte';
	import JobQueuePanel from '$lib/components/dashboard/JobQueuePanel.svelte';
	import ReadingHeatmap from '$lib/components/dashboard/ReadingHeatmap.svelte';
	import ReadingStatsWidget from '$lib/components/dashboard/ReadingStatsWidget.svelte';
	import RecentHighlights from '$lib/components/dashboard/RecentHighlights.svelte';
	import RecentlyAdded from '$lib/components/dashboard/RecentlyAdded.svelte';

	const SHELF_PAGE = { offset: { page: 1, pageSize: 12 } };
	const RECENT_PAGE = { offset: { page: 1, pageSize: 6 } };
	const JOB_PAGE = { offset: { page: 1, pageSize: 6 } };
	const HIGHLIGHTS = 5;

	const session = getHomeSession();
	const queryClient = useQueryClient();

	let span = $state<ReadingStatsSpan>('WEEK');
	// Ticks once a minute so the relative timestamps stay honest between fetches.
	let now = $state(new Date());
	let liveJobs = $state<Record<string, LiveJobProgress>>({});
	let queued = $state<number | null>(null);
	let queuedByKind = $state<Record<string, number>>({});
	let liveError = $state<string | null>(null);

	const greeting = $derived.by(() => {
		const hour = now.getHours();
		const time = hour < 5 ? 'Good night' : hour < 12 ? 'Good morning' : hour < 18 ? 'Good afternoon' : 'Good evening';
		return session.user ? `${time}, ${session.user.username}.` : `${time}.`;
	});

	const viewerQuery = createQuery(() => ({
		queryKey: ['dashboard-viewer'],
		queryFn: () => request(DashboardViewerDocument, {}),
		enabled: browser
	}));
	// `jobs` needs READ_JOBS; the server owner bypasses every permission guard.
	const canReadJobs = $derived(
		Boolean(
			viewerQuery.data?.me.isServerOwner || viewerQuery.data?.me.permissions.includes('READ_JOBS')
		)
	);

	const statsQuery = createQuery(() => ({
		queryKey: ['readingStats', span],
		queryFn: () => request(ReadingStatsDocument, { span }),
		enabled: browser
	}));
	// The heatmap is always the last 12 months, independent of the card span.
	const yearQuery = createQuery(() => ({
		queryKey: ['readingStats', 'YEAR'],
		queryFn: () => request(ReadingStatsDocument, { span: 'YEAR' }),
		enabled: browser
	}));
	const keepReadingQuery = createQuery(() => ({
		queryKey: ['keepReading'],
		queryFn: () => request(DashboardKeepReadingDocument, { pagination: SHELF_PAGE }),
		enabled: browser
	}));
	const recentQuery = createQuery(() => ({
		queryKey: ['recentlyAddedMedia'],
		queryFn: () => request(DashboardRecentlyAddedDocument, { pagination: RECENT_PAGE }),
		enabled: browser
	}));
	const devicesQuery = createQuery(() => ({
		queryKey: ['devices'],
		queryFn: () => request(DevicesDocument, {}),
		enabled: browser
	}));
	const highlightsQuery = createQuery(() => ({
		queryKey: ['annotations', 'recent', HIGHLIGHTS],
		queryFn: () => request(DashboardRecentAnnotationsDocument, { pageSize: HIGHLIGHTS }),
		enabled: browser
	}));
	const jobsQuery = createQuery(() => ({
		queryKey: ['dashboard-jobs'],
		queryFn: () => request(DashboardJobsDocument, { pagination: JOB_PAGE }),
		enabled: browser && canReadJobs
	}));

	const year = $derived(yearQuery.data?.readingStats);

	$effect(() => {
		const timer = setInterval(() => (now = new Date()), 60_000);
		return () => clearInterval(timer);
	});

	// One socket for every live surface: job progress, device sightings, and
	// provider source health. `readEvents` already carries all three, so a
	// second subscription would only cost a second WebSocket.
	$effect(() => {
		return subscribe(
			DashboardLiveEventsDocument,
			{},
			{
				next: (result) => {
					const event = result.data?.readEvents;
					if (!event) return;
					if (event.__typename === 'JobQueueStatus') {
						queued = event.count;
						queuedByKind = normalizeCounts(event.countByType);
						return;
					}
					if (event.__typename === 'JobUpdate') {
						liveJobs = {
							...liveJobs,
							[event.id]: {
								status: event.status ?? null,
								message: event.message ?? null,
								subtitle: event.subtitle ?? null,
								completedTasks: event.completedTasks ?? null,
								remainingTasks: event.remainingTasks ?? null
							}
						};
						return;
					}
					if (event.__typename === 'JobStarted' || event.__typename === 'JobOutput') {
						void queryClient.invalidateQueries({ queryKey: ['dashboard-jobs'] });
						return;
					}
					// A sighting usually means a sync landed: the device row, the
					// shelf, and the totals may all have moved.
					if (event.__typename === 'DeviceSeen') {
						now = new Date();
						void queryClient.invalidateQueries({ queryKey: ['devices'] });
						void queryClient.invalidateQueries({ queryKey: ['keepReading'] });
						void queryClient.invalidateQueries({ queryKey: ['readingStats'] });
						return;
					}
					if (event.__typename === 'ProviderSourceHealthChanged') {
						announceSourceHealth(event.name, event.healthStatus);
					}
				},
				error: () => {
					liveError = 'The event socket closed; job and device state refresh on reload.';
				},
				complete: () => undefined
			}
		);
	});

	/** `JobQueueStatus.countByType` is an untyped JSON object of kind → count. */
	function normalizeCounts(value: unknown): Record<string, number> {
		if (!value || typeof value !== 'object') return {};
		return Object.fromEntries(
			Object.entries(value as Record<string, unknown>)
				.map(([kind, count]) => [kind, Number(count)] as const)
				.filter(([, count]) => Number.isFinite(count) && count > 0)
		);
	}

	/**
	 * A source health event is only emitted on a transition, so `OK` is a
	 * recovery and anything else is a degradation worth interrupting for.
	 */
	function announceSourceHealth(name: string, status: string): void {
		if (status === 'OK') {
			toast.success(`${name} is reachable again.`);
		} else if (status === 'DEAD') {
			toast.error(`${name} is down; Coppice stopped listing it.`);
		} else {
			toast.warning(`${name} is degraded.`);
		}
	}
</script>

<svelte:head>
	<title>Dashboard · Coppice</title>
</svelte:head>

<div class="flex flex-col gap-8">
	<PageHeader title="Dashboard" description={greeting}>
		{#snippet actions()}
			<Button href={`${resolve('/connections')}#notifications`} size="sm" variant="outline">
				<BellIcon data-icon="inline-start" aria-hidden="true" />
				Notifications
			</Button>
		{/snippet}
	</PageHeader>

	<div class="grid grid-cols-1 gap-6 @3xl/page:grid-cols-12">
		<ReadingHeatmap
			days={year?.days ?? []}
			to={year?.to ?? ''}
			streakDays={year?.streakDays ?? 0}
			query={yearQuery}
			class="@3xl/page:col-span-8 @5xl/page:col-span-8"
		/>

		<DevicesWidget
			devices={devicesQuery.data?.devices ?? []}
			query={devicesQuery}
			{now}
			class="@3xl/page:col-span-4 @5xl/page:col-span-4"
		/>

		<ContinueReadingShelf
			books={keepReadingQuery.data?.keepReading.nodes ?? []}
			query={keepReadingQuery}
			{now}
			class="@3xl/page:col-span-12"
		/>

		<ReadingStatsWidget
			bind:span
			stats={statsQuery.data?.readingStats}
			query={statsQuery}
			class="@3xl/page:col-span-7 @5xl/page:col-span-8"
		/>

		<RecentHighlights
			annotations={highlightsQuery.data?.annotations.items ?? []}
			total={highlightsQuery.data?.annotations.total ?? 0}
			query={highlightsQuery}
			{now}
			class="@3xl/page:col-span-7"
		/>

		<RecentlyAdded
			books={recentQuery.data?.recentlyAddedMedia.nodes ?? []}
			query={recentQuery}
			{now}
			class="@3xl/page:col-span-5"
		/>

		{#if canReadJobs}
			<JobQueuePanel
				jobs={jobsQuery.data?.jobs.nodes ?? []}
				live={liveJobs}
				{queued}
				{queuedByKind}
				query={jobsQuery}
				{liveError}
				{now}
				class="@3xl/page:col-span-12"
			/>
		{/if}
	</div>
</div>
