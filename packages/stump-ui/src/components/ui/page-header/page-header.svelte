<script lang="ts">
	/**
	 * The top of a screen: one `<h1>`, an optional eyebrow above it, a lede
	 * under it, and the screen's primary actions on the trailing edge. Every
	 * route renders exactly one, so the document outline has a single
	 * heading level 1 and the spacing above the content is uniform.
	 */
	import type { Snippet } from "svelte";
	import type { HTMLAttributes } from "svelte/elements";
	import { cn, type WithElementRef } from "@stump/ui/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		eyebrow,
		title,
		description,
		actions,
		children,
		...restProps
	}: WithElementRef<HTMLAttributes<HTMLElement>> & {
		/** A small label above the title: the section this screen sits in. */
		eyebrow?: string;
		title: string;
		description?: string;
		/** Buttons on the trailing edge; they wrap under the copy when narrow. */
		actions?: Snippet;
	} = $props();
</script>

<header
	bind:this={ref}
	data-slot="page-header"
	class={cn("flex flex-wrap items-end justify-between gap-x-6 gap-y-3", className)}
	{...restProps}
>
	<div class="flex min-w-0 flex-col gap-1">
		{#if eyebrow}
			<span data-slot="page-header-eyebrow" class="text-xs font-medium tracking-wide text-muted-foreground uppercase">
				{eyebrow}
			</span>
		{/if}
		<h1 data-slot="page-header-title" class="text-2xl font-semibold tracking-tight text-balance">
			{title}
		</h1>
		{#if description}
			<p data-slot="page-header-description" class="max-w-2xl text-sm/relaxed text-muted-foreground text-pretty">
				{description}
			</p>
		{/if}
		{@render children?.()}
	</div>
	{#if actions}
		<div data-slot="page-header-actions" class="flex flex-wrap items-center gap-2">
			{@render actions()}
		</div>
	{/if}
</header>
