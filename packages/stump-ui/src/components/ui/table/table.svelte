<script lang="ts">
	/**
	 * The shared table, with an opt-in `stacked` mode for phones: below the
	 * `@md` width of its own container the header disappears and every row
	 * becomes a card. Cells that carry `data-label` render that label beside
	 * their value; cells without one (the title, the actions) run full width.
	 * A cell holding a checkbox keeps the first column so the selection box
	 * sits beside the title instead of on its own line.
	 */
	import { cn, type WithElementRef } from "@stump/ui/utils.js";
	import type { HTMLTableAttributes } from "svelte/elements";

	let {
		ref = $bindable(null),
		class: className,
		stacked = false,
		children,
		...restProps
	}: WithElementRef<HTMLTableAttributes> & { stacked?: boolean } = $props();

	const STACKED =
		"@max-md/table:block @max-md/table:[&_thead]:hidden @max-md/table:[&_tbody]:block " +
		"@max-md/table:[&_tr]:grid @max-md/table:[&_tr]:grid-cols-[auto_minmax(0,1fr)] @max-md/table:[&_tr]:gap-x-3 @max-md/table:[&_tr]:gap-y-1.5 @max-md/table:[&_tr]:px-3 @max-md/table:[&_tr]:py-3 " +
		"@max-md/table:[&_td]:col-start-2 @max-md/table:[&_td]:min-w-0 @max-md/table:[&_td]:p-0 @max-md/table:[&_td]:whitespace-normal " +
		"@max-md/table:[&_td:has([role=checkbox])]:col-start-1 @max-md/table:[&_td:has([role=checkbox])]:row-start-1 @max-md/table:[&_td:has([role=checkbox])]:pt-0.5 " +
		"@max-md/table:[&_td[data-label]]:flex @max-md/table:[&_td[data-label]]:items-baseline @max-md/table:[&_td[data-label]]:justify-between @max-md/table:[&_td[data-label]]:gap-3 @max-md/table:[&_td[data-label]]:text-right " +
		"@max-md/table:[&_td[data-label]]:before:shrink-0 @max-md/table:[&_td[data-label]]:before:text-left @max-md/table:[&_td[data-label]]:before:text-xs @max-md/table:[&_td[data-label]]:before:text-muted-foreground @max-md/table:[&_td[data-label]]:before:content-[attr(data-label)]";
</script>

<div data-slot="table-container" class="relative w-full overflow-x-auto @container/table">
	<table
		bind:this={ref}
		data-slot="table"
		data-stacked={stacked || undefined}
		class={cn("w-full caption-bottom text-sm", stacked && STACKED, className)}
		{...restProps}
	>
		{@render children?.()}
	</table>
</div>
