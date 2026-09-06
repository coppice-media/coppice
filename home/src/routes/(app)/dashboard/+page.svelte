<script lang="ts">
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import BellIcon from '@lucide/svelte/icons/bell';
	import { Button } from '@stump/ui/components/ui/button';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import * as Tabs from '@stump/ui/components/ui/tabs';
	import { toast } from 'svelte-sonner';
	import { request, subscribe } from '@stump/ui/graphql/client';
	import {
		DashboardJobsDocument,
		DashboardKeepReadingDocument,
		DashboardLiveEventsDocument,
		DashboardRecentlyAddedDocument,
		DashboardViewerDocument,
		DevicesDocument,
		ReadingStatsDocument,
		type ReadingStatsSpan
	} from '$lib/graphql/generated/graphql';
	import type { LiveJobProgress } from '$lib/dashboard';
	import { minutesLabel } from '$lib/format';
	import { getHomeSession } from '$lib/session.svelte';
	import BookShelf from '$lib/components/dashboard/BookShelf.svelte';
	import DeviceSyncPanel from '$lib/components/dashboard/DeviceSyncPanel.svelte';
	import JobQueuePanel from '$lib/components/dashboard/JobQueuePanel.svelte';
	import ReadingHeatmap from '$lib/components/dashboard/ReadingHeatmap.svelte';
	import StatCard from '$lib/components/dashboard/StatCard.svelte';

	const SPANS: { value: ReadingStatsSpan; label: string }[] = [
		{ value: 'DAY', label: 'Today' },
		{ value: 'WEEK', label: 'Week' },
		{ value: 'MONTH', label: 'Month' },
		{ value: 'YEAR', label: 'Year' },
		{ value: 'ALL_TIME', label: 'All time' }
	];
	const SHELF_PAGE = { offset: { page: 1, pageSize: 12 } };
	const JOB_PAGE = { offset: { page: 1, pageSize: 6 } };

	const session = getHomeSession();
	const queryClient = useQueryClient();

	let span = $state<ReadingStatsSpan>('WEEK');
	// Ticks once a minute so the relative timestamps stay honest between fetches.
	let now = $state(new Date());
	let liveJobs = $state<Record<string, LiveJobProgress>>({});
	let queued = $state<number | null>(null);
	let queuedByKind = $state<Record<string, number>>({});
	let liveError = $state<string | null>(null);

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
		queryFn: () => request(DashboardRecentlyAddedDocument, { pagination: SHELF_PAGE }),
		enabled: browser
	}));
	const devicesQuery = createQuery(() => ({
		queryKey: ['devices'],
		queryFn: () => request(DevicesDocument, {}),
		enabled: browser
	}));
	const jobsQuery = createQuery(() => ({
		queryKey: ['dashboard-jobs'],
		queryFn: () => request(DashboardJobsDocument, { pagination: JOB_PAGE }),
		enabled: browser && canReadJobs
	}));

	const stats = $derived(statsQuery.data?.readingStats);
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
					if (event.__typename === 'DeviceSeen') {
						now = new Date();
						void queryClient.invalidateQueries({ queryKey: ['devices'] });
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
			toast.error(`${name} is down; Stump stopped listing it.`);
		} else {
			toast.warning(`${name} is degraded.`);
		}
	}
</script>

<svelte:head>
	<title>Dashboard · Stump</title>
</svelte:head>

<div class="flex flex-col gap-6">
	<div class="flex flex-wrap items-center gap-3">
		<div class="mr-auto">
			<h1 class="text-2xl font-semibold tracking-tight">Dashboard</h1>
			<p class="text-sm text-muted-foreground">
				{session.user ? `Signed in as ${session.user.username}.` : 'Your server at a glance.'}
			</p>
		</div>
		<Button href={resolve('/settings/notifications')} size="sm" variant="outline">
			<BellIcon aria-hidden="true" />
			Notifications
		</Button>
	</div>

	<Tabs.Root value={span} onValueChange={(value) => (span = value as ReadingStatsSpan)}>
		<Tabs.List>
			{#each SPANS as option (option.value)}
				<Tabs.Trigger value={option.value}>{option.label}</Tabs.Trigger>
			{/each}
		</Tabs.List>
	</Tabs.Root>

	{#if statsQuery.isPending}
		<div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
			{#each { length: 4 } as _, index (index)}
				<Skeleton class="h-24 rounded-xl" />
			{/each}
		</div>
	{:else}
		<div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
			<StatCard
				label="Time read"
				value={minutesLabel(stats?.minutes ?? 0)}
				hint={stats?.streakDays ? `${stats.streakDays}-day streak` : undefined}
			/>
			<StatCard label="Pages" value={String(stats?.pages ?? 0)} />
			<StatCard label="Sessions" value={String(stats?.sessions ?? 0)} />
			<StatCard
				label="Books finished"
				value={String(stats?.booksFinished ?? 0)}
				hint={stats?.from ? `since ${stats.from}` : undefined}
			/>
		</div>
	{/if}

	<ReadingHeatmap
		days={year?.days ?? []}
		to={year?.to ?? ''}
		streakDays={year?.streakDays ?? 0}
		pending={yearQuery.isPending}
		error={yearQuery.error}
	/>

	<BookShelf
		title="Continue reading"
		description="Books with an open reading session, most recent first."
		books={keepReadingQuery.data?.keepReading.nodes ?? []}
		pending={keepReadingQuery.isPending}
		error={keepReadingQuery.error}
		errorTitle="Unable to load your open books"
		emptyTitle="Nothing in progress"
		emptyDescription="Open a book on any device and it shows up here."
		showProgress
		{now}
	/>

	<BookShelf
		title="Recently added"
		description="The newest books in the libraries you can see."
		books={recentQuery.data?.recentlyAddedMedia.nodes ?? []}
		pending={recentQuery.isPending}
		error={recentQuery.error}
		errorTitle="Unable to load recently added books"
		emptyTitle="No books yet"
		emptyDescription="Scan a library to fill this shelf."
		{now}
	/>

	<div class="grid gap-6 lg:grid-cols-2">
		<DeviceSyncPanel
			devices={devicesQuery.data?.devices ?? []}
			pending={devicesQuery.isPending}
			error={devicesQuery.error}
			{now}
		/>
		{#if canReadJobs}
			<JobQueuePanel
				jobs={jobsQuery.data?.jobs.nodes ?? []}
				live={liveJobs}
				{queued}
				{queuedByKind}
				pending={jobsQuery.isPending}
				error={jobsQuery.error}
				{liveError}
				{now}
			/>
		{/if}
	</div>
</div>
