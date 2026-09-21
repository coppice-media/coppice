<script lang="ts">
	/**
	 * A 53-week calendar strip of `readingStats(span: YEAR).days`, one cell
	 * per logical reading day, shaded by minutes or pages.
	 */
	import { resolve } from '$app/paths';
	import * as Tabs from '@stump/ui/components/ui/tabs';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import {
		buildHeatmap,
		dayLabel,
		HEATMAP_LEVEL_CLASSES,
		type ActivityDay,
		type HeatmapCell,
		type HeatmapMetric
	} from '$lib/dashboard';
	import { countNoun, minutesLabel } from '$lib/format';
	import Widget from './Widget.svelte';

	// Row labels: Mon / Wed / Fri, the convention every contribution grid uses.
	const ROW_LABELS = ['', 'Mon', '', 'Wed', '', 'Fri', ''];
	const LEVELS: HeatmapCell['level'][] = [0, 1, 2, 3, 4];

	let {
		days,
		to,
		streakDays,
		query,
		class: className
	}: {
		days: readonly ActivityDay[];
		/** The last counted day, i.e. `readingStats.to`; empty while loading. */
		to: string;
		streakDays: number;
		query: { isPending: boolean; error: unknown; refetch: () => unknown };
		class?: string;
	} = $props();

	let metric = $state<HeatmapMetric>('minutes');

	const columns = $derived(to ? buildHeatmap(days, to, metric) : []);
	const total = $derived(days.reduce((sum, day) => sum + day[metric], 0));
	const totalLabel = $derived(metric === 'minutes' ? minutesLabel(total) : countNoun(total, 'page'));

	function cellTitle(cell: HeatmapCell): string {
		if (!cell.date) return '';
		if (cell.sessions === 0) return `${dayLabel(cell.date)}: no reading`;
		return `${dayLabel(cell.date)}: ${minutesLabel(cell.minutes)}, ${countNoun(cell.pages, 'page')}, ${countNoun(cell.sessions, 'session')}`;
	}
</script>

<Widget
	title="Reading activity"
	description="{totalLabel} over the last 12 months · {streakDays > 0
		? `${streakDays}-day streak`
		: 'no current streak'}"
	href={resolve('/reading')}
	{query}
	empty={days.length === 0}
	emptyTitle="No reading sessions yet"
	emptyDescription="Read here or sync a device; every session lands on this calendar."
	errorTitle="Unable to load reading activity"
	class={className}
>
	{#snippet action()}
		<Tabs.Root value={metric} onValueChange={(value) => (metric = value as HeatmapMetric)}>
			<Tabs.List aria-label="Heatmap metric">
				<Tabs.Trigger value="minutes" class="px-2 text-xs">Minutes</Tabs.Trigger>
				<Tabs.Trigger value="pages" class="px-2 text-xs">Pages</Tabs.Trigger>
			</Tabs.List>
		</Tabs.Root>
	{/snippet}
	{#snippet skeleton()}
		<Skeleton class="h-32 w-full rounded-lg" />
	{/snippet}
	<div class="flex flex-col gap-2 overflow-x-auto pb-1">
		<div class="flex gap-[3px] pl-9 text-[10px] text-muted-foreground">
			{#each columns as column, index (index)}
				<span class="w-3 shrink-0">{column.monthLabel ?? ''}</span>
			{/each}
		</div>
		<div class="flex gap-[3px]">
			<div
				class="flex w-8 shrink-0 flex-col gap-[3px] text-right text-[10px] text-muted-foreground"
			>
				{#each ROW_LABELS as label, row (row)}
					<span class="h-3 leading-3">{label}</span>
				{/each}
			</div>
			<div class="flex gap-[3px]" role="img" aria-label="Reading activity over the last 12 months">
				{#each columns as column, index (index)}
					<div class="flex flex-col gap-[3px]">
						{#each column.cells as cell, row (row)}
							<span
								class="size-3 shrink-0 rounded-[3px] {cell.date
									? HEATMAP_LEVEL_CLASSES[cell.level]
									: 'bg-transparent'}"
								title={cellTitle(cell)}
							></span>
						{/each}
					</div>
				{/each}
			</div>
		</div>
		<div class="flex items-center gap-1.5 pl-9 text-[10px] text-muted-foreground">
			<span>Less</span>
			{#each LEVELS as level (level)}
				<span class="size-3 rounded-[3px] {HEATMAP_LEVEL_CLASSES[level]}"></span>
			{/each}
			<span>More</span>
		</div>
	</div>
</Widget>
