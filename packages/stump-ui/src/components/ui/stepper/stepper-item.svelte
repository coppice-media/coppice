<script lang="ts">
	/**
	 * One step. Its state is derived from the root's `current`, never passed
	 * in: a completed step shows a check instead of its number, the current
	 * one is `aria-current="step"`, and upcoming ones are muted.
	 */
	import type { HTMLLiAttributes } from "svelte/elements";
	import CheckIcon from "@lucide/svelte/icons/check";
	import { cn, type WithElementRef } from "@stump/ui/utils.js";
	import { getStepperCtx } from "./stepper.svelte";

	let {
		ref = $bindable(null),
		class: className,
		step,
		title,
		description,
		children,
		...restProps
	}: WithElementRef<HTMLLiAttributes, HTMLLIElement> & {
		/** This item's position, 1-based, matching the root's `current`. */
		step: number;
		title: string;
		description?: string;
	} = $props();

	const ctx = getStepperCtx();
	const state = $derived(step < ctx.current ? "complete" : step === ctx.current ? "current" : "upcoming");
</script>

<li
	bind:this={ref}
	data-slot="stepper-item"
	data-state={state}
	aria-current={state === "current" ? "step" : undefined}
	class={cn(
		"group/stepper-item flex min-w-0 flex-1 items-center gap-3 group-data-[orientation=vertical]/stepper:flex-none",
		className
	)}
	{...restProps}
>
	<span
		data-slot="stepper-indicator"
		class="flex size-7 shrink-0 items-center justify-center rounded-full border text-xs font-medium tabular-nums transition-colors group-data-[state=complete]/stepper-item:border-primary group-data-[state=complete]/stepper-item:bg-primary group-data-[state=complete]/stepper-item:text-primary-foreground group-data-[state=current]/stepper-item:border-primary group-data-[state=current]/stepper-item:text-foreground group-data-[state=upcoming]/stepper-item:text-muted-foreground [&_svg:not([class*='size-'])]:size-3.5"
	>
		{#if state === "complete"}
			<CheckIcon aria-hidden="true" />
		{:else}
			{step}
		{/if}
	</span>
	<span data-slot="stepper-copy" class="flex min-w-0 flex-col">
		<span
			data-slot="stepper-title"
			class="truncate text-sm font-medium group-data-[state=upcoming]/stepper-item:text-muted-foreground"
		>
			{title}
		</span>
		{#if description}
			<span data-slot="stepper-description" class="truncate text-xs text-muted-foreground">{description}</span>
		{/if}
		{@render children?.()}
	</span>
	<span
		data-slot="stepper-separator"
		aria-hidden="true"
		class="h-px flex-1 bg-border group-last/stepper-item:hidden group-data-[state=complete]/stepper-item:bg-primary group-data-[orientation=vertical]/stepper:hidden"
	></span>
</li>
