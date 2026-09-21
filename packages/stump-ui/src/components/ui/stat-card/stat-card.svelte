<script lang="ts" module>
	export type StatCardTrend = {
		direction: "up" | "down" | "flat";
		label: string;
	};
</script>

<script lang="ts">
	/**
	 * One figure with its label: the unit the statistics screens are built
	 * from. The label is the accessible name of the value, so it is a real
	 * `<dt>`/`<dd>` pair rather than two spans.
	 */
	import type { Snippet } from "svelte";
	import type { HTMLAttributes } from "svelte/elements";
	import TrendingDownIcon from "@lucide/svelte/icons/trending-down";
	import TrendingUpIcon from "@lucide/svelte/icons/trending-up";
	import MinusIcon from "@lucide/svelte/icons/minus";
	import { cn, type WithoutChildren, type WithElementRef } from "@stump/ui/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		label,
		value,
		hint,
		trend,
		icon,
		...restProps
	}: WithoutChildren<WithElementRef<HTMLAttributes<HTMLDivElement>>> & {
		label: string;
		value: string | number;
		/** The sentence under the figure: what it counts, or why it is zero. */
		hint?: string;
		/** Movement against the previous span. `flat` is neutral, not bad. */
		trend?: StatCardTrend;
		icon?: Snippet;
	} = $props();

	const TrendIcon = $derived(
		trend?.direction === "up" ? TrendingUpIcon : trend?.direction === "down" ? TrendingDownIcon : MinusIcon
	);
</script>

<div
	bind:this={ref}
	data-slot="stat-card"
	class={cn(
		"flex flex-col gap-1 rounded-xl border bg-card p-4 text-card-foreground ring-1 ring-foreground/5",
		className
	)}
	{...restProps}
>
	<div class="flex items-start justify-between gap-2">
		<dl class="flex min-w-0 flex-col gap-1">
			<dt data-slot="stat-card-label" class="text-xs font-medium tracking-wide text-muted-foreground uppercase">
				{label}
			</dt>
			<dd data-slot="stat-card-value" class="truncate text-2xl font-semibold tabular-nums">
				{value}
			</dd>
		</dl>
		{#if icon}
			<span
				data-slot="stat-card-icon"
				class="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground [&_svg:not([class*='size-'])]:size-4"
			>
				{@render icon()}
			</span>
		{/if}
	</div>
	{#if trend}
		<p
			data-slot="stat-card-trend"
			data-direction={trend.direction}
			class="flex items-center gap-1 text-xs text-muted-foreground data-[direction=down]:text-destructive [&_svg:not([class*='size-'])]:size-3.5"
		>
			<TrendIcon aria-hidden="true" />
			{trend.label}
		</p>
	{/if}
	{#if hint}
		<p data-slot="stat-card-hint" class="text-xs text-muted-foreground text-pretty">{hint}</p>
	{/if}
</div>
