<script lang="ts">
	import { browser } from '$app/environment'
	import { resolve } from '$app/paths'
	import { page } from '$app/state'
	import { createQuery } from '@tanstack/svelte-query'
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Cover } from '@stump/ui/components/ui/cover';
	import { PageHeader } from '@stump/ui/components/ui/page-header';
	import { QueryState } from '@stump/ui/components/ui/query-state';
	import { StatCard } from '@stump/ui/components/ui/stat-card';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import * as Tabs from '@stump/ui/components/ui/tabs';
	import XIcon from '@lucide/svelte/icons/x';
	import { request } from '@stump/ui/graphql/client';
	import {
		ConsoleBooksDocument,
		DashboardKeepReadingDocument,
		ReadingStatsDocument,
		type ConsoleBookRowFragment,
		type MediaFilterInput,
		type MediaOrderBy,
		type ReadingStatsSpan
	} from '$lib/graphql/generated/graphql';
	import { absoluteTime, countLabel, countNoun, minutesLabel, relativeTime } from '$lib/format';
	import { dayLabel, SPAN_OPTIONS } from '$lib/dashboard';
	import { DEVICE_KIND_LABELS } from '$lib/devices';
	import { offsetPage } from '$lib/library';
	import ContinueReadingCard from '$lib/components/dashboard/ContinueReadingCard.svelte';
	import Pager from '$lib/components/library/Pager.svelte';

	const CURRENT_PAGE_SIZE = 24;
	const HISTORY_PAGE_SIZE = 18;
	const HISTORY_FILTER: MediaFilterInput = {
		readingStatus: { isAnyOf: ['FINISHED', 'ABANDONED'] }
	};
	const HISTORY_ORDER: MediaOrderBy[] = [
		{ media: { field: 'UPDATED_AT', direction: 'DESC' } }
	];

	type ReadingView = 'current' | 'history';

	let view = $state<ReadingView>('current');
	let span = $state<ReadingStatsSpan>('MONTH');
	let historyPage = $state(1);
	let now = $state(new Date());
	const deviceId = $derived(page.url.searchParams.get('device'))

	const statsQuery = createQuery(() => ({
		queryKey: ['readingStats', span, deviceId],
		queryFn: () => request(ReadingStatsDocument, { span, deviceId }),
		enabled: browser
	}))
	const currentQuery = createQuery(() => ({
		queryKey: ['reading-current'],
		queryFn: () =>
			request(DashboardKeepReadingDocument, {
				pagination: { offset: { page: 1, pageSize: CURRENT_PAGE_SIZE } }
			}),
		enabled: browser
	}));
	const historyQuery = createQuery(() => ({
		queryKey: ['reading-history', historyPage],
		queryFn: () =>
			request(ConsoleBooksDocument, {
				filter: HISTORY_FILTER,
				orderBy: HISTORY_ORDER,
				pagination: { offset: { page: historyPage, pageSize: HISTORY_PAGE_SIZE } }
			}),
		enabled: browser
	}));

	const stats = $derived(statsQuery.data?.readingStats)
	const currentBooks = $derived(currentQuery.data?.keepReading.nodes ?? [])
	const historyBooks = $derived(historyQuery.data?.media.nodes ?? [])
	const historyPageInfo = $derived(offsetPage(historyQuery.data?.media.pageInfo))
	const focusedDeviceName = $derived(
		deviceId ? (stats?.devices.find((device) => device.deviceId === deviceId)?.name ?? deviceId) : null
	)
	const busiestMinutes = $derived(Math.max(1, ...(stats?.days.map((day) => day.minutes) ?? [1])))

	$effect(() => {
		const timer = setInterval(() => (now = new Date()), 60_000);
		return () => clearInterval(timer);
	});

	function selectSpan(value: string): void {
		if (SPAN_OPTIONS.some((option) => option.value === value)) {
			span = value as ReadingStatsSpan;
		}
	}

	function selectView(value: string): void {
		if (value === 'current' || value === 'history') view = value;
	}

	function turnHistoryPage(next: number): void {
		historyPage = Math.max(1, next);
	}

	function latestRecord(book: ConsoleBookRowFragment) {
		// `readHistory` is newest first (`ReadthroughRecordLoader` contract).
		return book.readHistory[0] ?? null;
	}
</script>

<svelte:head>
	<title>Reading · Coppice</title>
	<meta
		name="description"
		content="Review current reading progress, reading sessions, and completed readthroughs."
	/>
</svelte:head>

{#snippet currentSkeleton()}
	<div class="grid grid-cols-1 gap-4 @sm/page:grid-cols-2 @2xl/page:grid-cols-3 @4xl/page:grid-cols-4">
		{#each { length: 6 } as _, index (index)}
			<div class="flex flex-col gap-3">
				<Skeleton class="aspect-2/3 w-full rounded-lg" />
				<Skeleton class="h-4 w-4/5" />
				<Skeleton class="h-3 w-2/5" />
				<Skeleton class="h-2 w-full" />
				<Skeleton class="h-9 w-full rounded-md" />
			</div>
		{/each}
	</div>
{/snippet}

{#snippet historySkeleton()}
	<div class="flex flex-col divide-y">
		{#each { length: 5 } as _, index (index)}
			<div class="flex items-center gap-3 py-3">
				<Skeleton class="h-20 w-14 shrink-0 rounded-md" />
				<div class="flex min-w-0 flex-1 flex-col gap-2">
					<Skeleton class="h-4 w-3/5" />
					<Skeleton class="h-3 w-2/5" />
					<Skeleton class="h-5 w-20 rounded-md" />
				</div>
				<Skeleton class="h-8 w-14 rounded-md" />
			</div>
		{/each}
	</div>
{/snippet}

<div class="flex flex-col gap-8">
	<PageHeader
		title="Reading"
		description={focusedDeviceName
			? `Activity and progress for ${focusedDeviceName}. Current reading and history lists remain account-wide.`
			: 'Your current progress, reading sessions, and completed readthroughs.'}
	>
		{#snippet actions()}
			<Button href={resolve('/library')} size="sm" variant="outline">Browse library</Button>
		{/snippet}
	</PageHeader>

	<div class="flex flex-wrap items-center gap-3">
		<Tabs.Root value={view} onValueChange={selectView}>
			<Tabs.List aria-label="Reading view" class="max-w-full overflow-x-auto">
				<Tabs.Trigger value="current">Current reading</Tabs.Trigger>
				<Tabs.Trigger value="history">History</Tabs.Trigger>
			</Tabs.List>
		</Tabs.Root>
		<div class="flex min-w-0 flex-wrap items-center gap-2">
			<span class="text-xs font-medium text-muted-foreground">Stats</span>
			<Tabs.Root value={span} onValueChange={selectSpan}>
				<Tabs.List aria-label="Statistics span" class="max-w-full overflow-x-auto">
					{#each SPAN_OPTIONS as option (option.value)}
						<Tabs.Trigger value={option.value} class="px-2 text-xs">{option.label}</Tabs.Trigger>
					{/each}
				</Tabs.List>
			</Tabs.Root>
		</div>
		{#if deviceId}
			<Button size="sm" variant="outline" href={resolve('/reading')}>
				<XIcon aria-hidden="true" />
				Clear device filter
			</Button>
		{/if}
	</div>
	{#if focusedDeviceName}
		<p class="rounded-lg border border-dashed bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
			Statistics are filtered to <span class="font-medium text-foreground">{focusedDeviceName}</span>.
			The current-reading and history lists below remain account-wide.
		</p>
	{/if}

	<section aria-labelledby="reading-summary-heading">
		<h2 id="reading-summary-heading" class="sr-only">Reading summary</h2>
		<QueryState
			query={statsQuery}
			empty={!stats}
			emptyTitle="No reading summary yet"
			emptyDescription="Read a book here or sync a device to start building your history."
			errorTitle="Unable to load reading statistics"
			rows={4}
		>
			<div class="grid grid-cols-2 gap-3 @2xl/page:grid-cols-4">
				<StatCard label="Time read" value={minutesLabel(stats?.minutes ?? 0)} hint={countNoun(stats?.sessions ?? 0, 'session')} />
				<StatCard label="Sessions" value={countLabel(stats?.sessions ?? 0)} hint="Reading intervals in this span" />
				<StatCard label="Pages" value={countLabel(stats?.pages ?? 0)} hint="Pages turned in this span" />
				<StatCard label="Books finished" value={countLabel(stats?.booksFinished ?? 0)} hint={stats?.booksFinished ? 'Completed readthroughs' : 'None completed yet'} />
			</div>
		</QueryState>
	</section>

	{#if view === 'current'}
		<section aria-labelledby="current-reading-heading" class="flex flex-col gap-4">
			<div class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
				<h2 id="current-reading-heading" class="text-lg font-semibold tracking-tight">Current reading</h2>
				<p class="text-sm text-muted-foreground">
					{currentBooks.length
						? `${countLabel(currentBooks.length)} ${currentBooks.length === 1 ? 'book' : 'books'} in progress`
						: 'Books with an active reading head appear here.'}
				</p>
			</div>
			<QueryState
				query={currentQuery}
				empty={currentBooks.length === 0}
				emptyTitle="Nothing in progress"
				emptyDescription="Start a book here or sync a reading device to see its progress."
				errorTitle="Unable to load current reading"
				skeleton={currentSkeleton}
			>
				<ul class="grid grid-cols-1 gap-5 @sm/page:grid-cols-2 @2xl/page:grid-cols-3 @4xl/page:grid-cols-4">
					{#each currentBooks as book (book.id)}
						<li class="h-full min-w-0">
							<ContinueReadingCard {book} {now} class="w-full" />
						</li>
					{/each}
				</ul>
			</QueryState>
		</section>
	{:else}
		<section aria-labelledby="reading-history-heading" class="flex flex-col gap-4">
			<div class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
				<h2 id="reading-history-heading" class="text-lg font-semibold tracking-tight">Reading history</h2>
				<p class="text-sm text-muted-foreground">
					{#if historyPageInfo}
						{countNoun(historyPageInfo.totalItems, 'book')} with a completed or abandoned readthrough
					{:else}
						Completed and abandoned readthroughs from your visible libraries.
					{/if}
				</p>
			</div>
			<QueryState
				query={historyQuery}
				empty={historyBooks.length === 0}
				emptyTitle="No completed readthroughs"
				emptyDescription="Finished and abandoned books will appear here after you read or sync them."
				errorTitle="Unable to load reading history"
				skeleton={historySkeleton}
			>
				<ul class="flex flex-col divide-y rounded-xl border bg-card px-4">
					{#each historyBooks as book (book.id)}
						{@const record = latestRecord(book)}
						<li class="flex min-w-0 items-center gap-3 py-3 sm:gap-4">
							<a
								class="block shrink-0"
								href={resolve('/(app)/reader/[mediaId]', { mediaId: book.id })}
								tabindex="-1"
								aria-hidden="true"
							>
								<Cover src={book.thumbnail.url} class="h-20 w-14 rounded-md ring-1 ring-foreground/10 sm:h-24 sm:w-16">
									{#snippet fallback()}
										<span class="px-1 text-center text-[10px] uppercase">{book.extension || 'Book'}</span>
									{/snippet}
								</Cover>
							</a>
							<div class="flex min-w-0 flex-1 flex-col gap-1">
								<a
									class="truncate text-sm font-medium hover:underline"
									href={resolve('/(app)/reader/[mediaId]', { mediaId: book.id })}
								>
									{book.resolvedName}
								</a>
								<a
									class="truncate text-xs text-muted-foreground hover:underline"
									href={resolve('/(app)/series/[id]', { id: book.series.id })}
								>
									{book.series.resolvedName}
								</a>
								<div class="flex flex-wrap items-center gap-2">
									<Badge variant={record?.dnf ? 'destructive' : 'secondary'}>
										{record ? (record.dnf ? 'Abandoned' : 'Finished') : 'History'}
									</Badge>
									{#if book.readHistory.length > 1}
										<span class="text-xs text-muted-foreground">
											{countNoun(book.readHistory.length, 'readthrough')}
										</span>
									{/if}
								</div>
							</div>
							{#if record}
								<time
									class="hidden shrink-0 text-right text-xs text-muted-foreground sm:block"
									datetime={record.completedAt}
									title={absoluteTime(record.completedAt)}
								>
									{relativeTime(record.completedAt, now)}
								</time>
							{/if}
							<Button
								href={resolve('/(app)/reader/[mediaId]', { mediaId: book.id })}
								size="sm"
								variant="ghost"
								aria-label={`Open ${book.resolvedName}`}
							>
								Open
							</Button>
						</li>
					{/each}
				</ul>
			</QueryState>
			{#if !historyQuery.isPending && !historyQuery.isError}
				<Pager page={historyPageInfo} noun="book" onpage={turnHistoryPage} />
			{/if}
		</section>
	{/if}

	{#if stats}
		<Card>
			<CardHeader>
				<CardTitle class="text-base">Daily activity</CardTitle>
				<CardDescription>
					{#if stats.from}
						{dayLabel(stats.from)} – {dayLabel(stats.to)} ·
					{:else}
						No recorded days in this span ·
					{/if}
					{stats.streakDays > 0 ? `${stats.streakDays}-day streak` : 'no current streak'}
				</CardDescription>
			</CardHeader>
			<CardContent class="flex flex-col gap-5">
				{#if stats.days.length === 0}
					<p class="text-sm text-muted-foreground">No reading sessions in this span.</p>
				{:else}
					<ol class="flex h-36 items-end gap-1 overflow-x-auto pb-1" aria-label="Minutes read per day">
						{#each stats.days as day (day.date)}
							{@const height = Math.max(4, Math.round((day.minutes / busiestMinutes) * 100))}
							<li
								class="min-w-2 max-w-8 flex-1 rounded-t bg-primary/80"
								style="height: {height}%"
								aria-label={`${dayLabel(day.date)}: ${minutesLabel(day.minutes)}, ${countNoun(day.pages, 'page')}, ${countNoun(day.sessions, 'session')}`}
								title={`${dayLabel(day.date)}: ${minutesLabel(day.minutes)}, ${countNoun(day.pages, 'page')}, ${countNoun(day.sessions, 'session')}`}
							></li>
						{/each}
					</ol>
				{/if}

				<div class="flex flex-col gap-2">
					<h3 class="text-sm font-medium">By device</h3>
					{#if stats.devices.length === 0}
						<p class="text-sm text-muted-foreground">No device sessions reported in this span.</p>
					{:else}
						<div class="overflow-x-auto">
							<table class="w-full min-w-[30rem] text-sm">
								<thead class="text-left text-muted-foreground">
									<tr>
										<th class="py-1 font-medium">Device</th>
										<th class="py-1 text-right font-medium">Sessions</th>
										<th class="py-1 text-right font-medium">Time</th>
										<th class="py-1 text-right font-medium">Pages</th>
									</tr>
								</thead>
								<tbody>
									{#each stats.devices as device (device.deviceId)}
										<tr class="border-t">
											<td class="py-2">
												{device.name ?? 'Removed device'}
												{#if device.kind}
													<span class="text-muted-foreground"> · {DEVICE_KIND_LABELS[device.kind]}</span>
												{/if}
											</td>
											<td class="py-2 text-right tabular-nums">{countLabel(device.sessions)}</td>
											<td class="py-2 text-right tabular-nums">{minutesLabel(device.minutes)}</td>
											<td class="py-2 text-right tabular-nums">{countLabel(device.pages)}</td>
										</tr>
									{/each}
								</tbody>
							</table>
						</div>
					{/if}
				</div>
			</CardContent>
		</Card>
	{/if}
</div>
