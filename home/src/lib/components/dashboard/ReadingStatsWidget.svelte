<script lang="ts">
	/**
	 * `readingStats` for a selectable span as four figures. Sessions are
	 * activity intervals from every lane — the reader here, Kobo, KOReader,
	 * Komga clients, and closed Liseur sessions — so the totals are the whole
	 * account, not one device.
	 */
	import { resolve } from '$app/paths';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { StatCard } from '@stump/ui/components/ui/stat-card';
	import * as Tabs from '@stump/ui/components/ui/tabs';
	import type { ReadingStatsQuery, ReadingStatsSpan } from '$lib/graphql/generated/graphql';
	import { SPAN_OPTIONS, SPAN_PHRASES } from '$lib/dashboard';
	import { countNoun, minutesLabel } from '$lib/format';
	import Widget from './Widget.svelte';

	let {
		stats,
		span = $bindable(),
		query,
		class: className
	}: {
		stats: ReadingStatsQuery['readingStats'] | undefined;
		span: ReadingStatsSpan;
		query: { isPending: boolean; error: unknown; refetch: () => unknown };
		class?: string;
	} = $props();

	const summaryDescription = $derived.by(() => {
		if (!stats) return undefined;
		const range = stats.from ? `${stats.from} to ${stats.to}` : `Through ${stats.to}`;
		return `${range} · ${stats.streakDays > 0 ? `${stats.streakDays}-day streak` : 'no current streak'}`;
	});
</script>

<Widget
	title="Reading {SPAN_PHRASES[span]}"
	description={summaryDescription}
	href={resolve('/reading')}
	{query}
	errorTitle="Unable to load reading statistics"
	class={className}
>
	{#snippet action()}
		<Tabs.Root value={span} onValueChange={(value) => (span = value as ReadingStatsSpan)}>
			<Tabs.List aria-label="Statistics span">
				{#each SPAN_OPTIONS as option (option.value)}
					<Tabs.Trigger value={option.value} class="px-2 text-xs">{option.label}</Tabs.Trigger>
				{/each}
			</Tabs.List>
		</Tabs.Root>
	{/snippet}
	{#snippet skeleton()}
		<div class="grid grid-cols-2 gap-3 @2xl/widget:grid-cols-4">
			{#each { length: 4 } as _, index (index)}
				<Skeleton class="h-24 rounded-xl" />
			{/each}
		</div>
	{/snippet}
	<div class="grid grid-cols-2 gap-3 @2xl/widget:grid-cols-4">
		<StatCard
			label="Time read"
			value={minutesLabel(stats?.minutes ?? 0)}
			hint="Across every reading session"
		/>
		<StatCard
			label="Sessions"
			value={stats?.sessions ?? 0}
			hint={countNoun(stats?.sessions ?? 0, 'reading interval')}
		/>
		<StatCard label="Pages" value={stats?.pages ?? 0} hint="Turned in every book" />
		<StatCard
			label="Books finished"
			value={stats?.booksFinished ?? 0}
			hint={stats?.booksFinished ? 'Completed readthroughs' : 'None completed yet'}
		/>
	</div>
</Widget>
