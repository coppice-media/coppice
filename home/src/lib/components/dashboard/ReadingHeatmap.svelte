<script lang="ts">
	/**
	 * A calendar strip of `readingStats(span: YEAR).days`, one cell per logical
	 * reading day, shaded by minutes or pages. The grid is as wide as the card:
	 * columns are `1fr` tracks and the number of weeks steps down (53 → 26 →
	 * 13) once the measured width would squeeze a column under ten pixels, so
	 * nothing ever scrolls sideways.
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
	/** Week counts the strip can show, widest first, and their span labels. */
	const SPANS: { weeks: number; label: string }[] = [
		{ weeks: 53, label: '12 months' },
		{ weeks: 26, label: '6 months' },
		{ weeks: 13, label: '3 months' }
	];
	/** The narrowest column (cell plus gap) worth rendering. */
	const MIN_COLUMN_PX = 10;
	/** The weekday label column (1.5rem) plus its gap. */
	const LABEL_COLUMN_PX = 27;

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
	let stripWidth = $state(0);

	const span = $derived.by(() => {
		const available = stripWidth - LABEL_COLUMN_PX;
		return (
			SPANS.find(({ weeks }) => stripWidth === 0 || available >= weeks * MIN_COLUMN_PX) ??
			SPANS[SPANS.length - 1]
		);
	});
	const columns = $derived(to ? buildHeatmap(days, to, metric, span.weeks) : []);
	const firstDate = $derived(columns[0]?.cells[0]?.date ?? '');
	const total = $derived(
		days.reduce((sum, day) => (day.date >= firstDate ? sum + day[metric] : sum), 0)
	);
	const totalLabel = $derived(metric === 'minutes' ? minutesLabel(total) : countNoun(total, 'page'));

	function cellTitle(cell: HeatmapCell): string {
		if (!cell.date) return '';
		if (cell.sessions === 0) return `${dayLabel(cell.date)}: no reading`;
		return `${dayLabel(cell.date)}: ${minutesLabel(cell.minutes)}, ${countNoun(cell.pages, 'page')}, ${countNoun(cell.sessions, 'session')}`;
	}

	/**
	 * Month labels need about three columns of room, so a partial first month
	 * and the last couple of columns stay blank rather than overlap.
	 */
	const monthLabels = $derived.by(() => {
		const labelled = columns.flatMap((column, index) => (column.monthLabel ? [index] : []));
		return columns.map((column, index) => {
			const next = labelled.find((candidate) => candidate > index) ?? Number.POSITIVE_INFINITY;
			return column.monthLabel && next - index >= 3 && index <= columns.length - 3
				? column.monthLabel
				: '';
		});
	});
</script>

<Widget
	title="Reading activity"
	description="{totalLabel} over the last {span.label} · {streakDays > 0
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
		<Skeleton class="h-36 w-full rounded-lg" />
	{/snippet}
	<div class="flex min-w-0 flex-col gap-2" bind:clientWidth={stripWidth}>
		<div
			class="grid gap-[3px] text-[10px] leading-none text-muted-foreground"
			style:grid-template-columns="1.5rem repeat({columns.length}, minmax(0, 1fr))"
			role="img"
			aria-label="Reading activity over the last {span.label}"
		>
			<span></span>
			{#each monthLabels as label, index (index)}
				<span class="h-3 overflow-visible whitespace-nowrap">{label}</span>
			{/each}
			{#each ROW_LABELS as label, row (row)}
				<span class="flex items-center justify-end pr-1">{label}</span>
				{#each columns as column, index (index)}
					{@const cell = column.cells[row]}
					<span
						class="aspect-square w-full rounded-[2px] {cell.date
							? HEATMAP_LEVEL_CLASSES[cell.level]
							: 'bg-transparent'}"
						title={cellTitle(cell)}
					></span>
				{/each}
			{/each}
		</div>
		<div class="flex items-center gap-1.5 pl-8 text-[10px] text-muted-foreground">
			<span>Less</span>
			{#each LEVELS as level (level)}
				<span class="size-3 rounded-[2px] {HEATMAP_LEVEL_CLASSES[level]}"></span>
			{/each}
			<span>More</span>
		</div>
	</div>
</Widget>
