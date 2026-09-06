<script lang="ts">
	import { browser } from '$app/environment';
	import { createQuery } from '@tanstack/svelte-query';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import * as Tabs from '@stump/ui/components/ui/tabs';
	import { request } from '@stump/ui/graphql/client';
	import { ReadingStatsDocument, type ReadingStatsSpan } from '$lib/graphql/generated/graphql';
	import { DEVICE_KIND_LABELS } from '$lib/devices';
	import { minutesLabel } from '$lib/format';

	const SPANS: { value: ReadingStatsSpan; label: string }[] = [
		{ value: 'WEEK', label: '7 days' },
		{ value: 'MONTH', label: '30 days' },
		{ value: 'QUARTER', label: '90 days' },
		{ value: 'YEAR', label: 'Year' },
		{ value: 'ALL_TIME', label: 'All time' }
	];

	let span = $state<ReadingStatsSpan>('MONTH');

	const statsQuery = createQuery(() => ({
		queryKey: ['readingStats', span],
		queryFn: () => request(ReadingStatsDocument, { span }),
		enabled: browser
	}));
	const stats = $derived(statsQuery.data?.readingStats);
	const busiestMinutes = $derived(Math.max(1, ...(stats?.days.map((day) => day.minutes) ?? [1])));

	function selectSpan(value: string): void {
		span = value as ReadingStatsSpan;
	}
</script>

<svelte:head>
	<title>Reading · Stump</title>
</svelte:head>

<div class="flex flex-col gap-6">
	<div class="flex flex-wrap items-center gap-3">
		<div class="mr-auto">
			<h1 class="text-2xl font-semibold tracking-tight">Reading</h1>
			<p class="text-sm text-muted-foreground">Sessions reported by your devices.</p>
		</div>
		<Tabs.Root value={span} onValueChange={selectSpan}>
			<Tabs.List>
				{#each SPANS as option (option.value)}
					<Tabs.Trigger value={option.value}>{option.label}</Tabs.Trigger>
				{/each}
			</Tabs.List>
		</Tabs.Root>
	</div>

	{#if statsQuery.isPending}
		<div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
			{#each { length: 4 } as _, index (index)}
				<Skeleton class="h-24 rounded-xl" />
			{/each}
		</div>
	{:else if statsQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load reading statistics</AlertTitle>
			<AlertDescription>
				{statsQuery.error instanceof Error ? statsQuery.error.message : 'Request failed.'}
			</AlertDescription>
		</Alert>
	{:else if stats}
		<div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
			{#each [['Time read', minutesLabel(stats.minutes)], ['Pages', String(stats.pages)], ['Sessions', String(stats.sessions)], ['Books finished', String(stats.booksFinished)]] as [label, value] (label)}
				<Card>
					<CardHeader class="pb-2">
						<CardDescription>{label}</CardDescription>
						<CardTitle class="text-3xl tabular-nums">{value}</CardTitle>
					</CardHeader>
				</Card>
			{/each}
		</div>

		<Card>
			<CardHeader>
				<CardTitle class="text-base">Daily activity</CardTitle>
				<CardDescription>
					{#if stats.streakDays > 0}
						{stats.streakDays}-day streak.
					{:else}
						No current streak.
					{/if}
					{#if stats.from}
						Counting from {stats.from} to {stats.to}.
					{/if}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{#if stats.days.length === 0}
					<p class="text-sm text-muted-foreground">No reading sessions in this span.</p>
				{:else}
					<ol class="flex h-32 items-end gap-1" aria-label="Minutes read per day">
						{#each stats.days as day (day.date)}
							<li
								class="min-w-1 max-w-6 flex-1 rounded-t bg-primary/80"
								style="height: {Math.max(4, Math.round((day.minutes / busiestMinutes) * 100))}%"
								title={`${day.date}: ${minutesLabel(day.minutes)}, ${day.pages} pages, ${day.sessions} sessions`}
							></li>
						{/each}
					</ol>
				{/if}
			</CardContent>
		</Card>

		{#if stats.devices.length}
			<Card>
				<CardHeader>
					<CardTitle class="text-base">By device</CardTitle>
				</CardHeader>
				<CardContent>
					<table class="w-full text-sm">
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
									<td class="py-1.5">
										{device.name ?? 'Removed device'}
										{#if device.kind}
											<span class="text-muted-foreground">· {DEVICE_KIND_LABELS[device.kind]}</span>
										{/if}
									</td>
									<td class="py-1.5 text-right tabular-nums">{device.sessions}</td>
									<td class="py-1.5 text-right tabular-nums">{minutesLabel(device.minutes)}</td>
									<td class="py-1.5 text-right tabular-nums">{device.pages}</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</CardContent>
			</Card>
		{/if}
	{/if}
</div>
