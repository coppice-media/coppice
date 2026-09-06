<script lang="ts">
	import * as Tabs from '@stump/ui/components/ui/tabs';
	import {
		buildHeatmap,
		dayLabel,
		HEATMAP_LEVEL_CLASSES,
		type ActivityDay,
		type HeatmapCell,
		type HeatmapMetric
	} from '$lib/dashboard';
	import { minutesLabel } from '$lib/format';
	import DashboardSection from './DashboardSection.svelte';

	// Row labels: Mon / Wed / Fri, the convention every contribution grid uses.
	const ROW_LABELS = ['', 'Mon', '', 'Wed', '', 'Fri', ''];
	const LEVELS: HeatmapCell['level'][] = [0, 1, 2, 3, 4];

	let {
		days,
		to,
		streakDays,
		pending = false,
		error = null
	}: {
		days: readonly ActivityDay[];
		/** The last counted day, i.e. `readingStats.to`; empty while loading. */
		to: string;
		streakDays: number;
		pending?: boolean;
		error?: unknown;
	} = $props();

	let metric = $state<HeatmapMetric>('minutes');

	const columns = $derived(to ? buildHeatmap(days, to, metric) : []);
	const total = $derived(days.reduce((sum, day) => sum + day[metric], 0));
	const totalLabel = $derived(metric === 'minutes' ? minutesLabel(total) : `${total} pages`);

	function cellTitle(cell: HeatmapCell): string {
		if (!cell.date) return '';
		if (cell.sessions === 0) return `${dayLabel(cell.date)}: no reading`;
		const sessions = `${cell.sessions} ${cell.sessions === 1 ? 'session' : 'sessions'}`;
		return `${dayLabel(cell.date)}: ${minutesLabel(cell.minutes)}, ${cell.pages} pages, ${sessions}`;
	}
</script>

<DashboardSection
	title="Reading activity"
	description="{streakDays > 0
		? `${streakDays}-day streak`
		: 'No current streak'} · {totalLabel} in the last 12 months"
	{pending}
	{error}
	errorTitle="Unable to load reading activity"
	emptyTitle="No reading sessions yet"
	emptyDescription="Register a device and sync your progress; every session shows up here."
	empty={days.length === 0}
	skeletonRows={3}
>
	{#snippet action()}
		<Tabs.Root value={metric} onValueChange={(value) => (metric = value as HeatmapMetric)}>
			<Tabs.List>
				<Tabs.Trigger value="minutes">Minutes</Tabs.Trigger>
				<Tabs.Trigger value="pages">Pages</Tabs.Trigger>
			</Tabs.List>
		</Tabs.Root>
	{/snippet}
	<div class="flex flex-col gap-2 overflow-x-auto pb-1">
		<div class="flex gap-1 pl-9 text-[10px] text-muted-foreground">
			{#each columns as column, index (index)}
				<span class="w-3 shrink-0">{column.monthLabel ?? ''}</span>
			{/each}
		</div>
		<div class="flex gap-1">
			<div class="flex w-8 shrink-0 flex-col gap-1 text-right text-[10px] text-muted-foreground">
				{#each ROW_LABELS as label, row (row)}
					<span class="h-3 leading-3">{label}</span>
				{/each}
			</div>
			<div class="flex gap-1" role="img" aria-label="Reading activity over the last 12 months">
				{#each columns as column, index (index)}
					<div class="flex flex-col gap-1">
						{#each column.cells as cell, row (row)}
							<span
								class="size-3 shrink-0 rounded-[2px] {cell.date
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
				<span class="size-3 rounded-[2px] {HEATMAP_LEVEL_CLASSES[level]}"></span>
			{/each}
			<span>More</span>
		</div>
	</div>
</DashboardSection>
