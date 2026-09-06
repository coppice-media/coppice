<script lang="ts">
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import { Button } from '@stump/ui/components/ui/button';
	import { countNoun } from '$lib/format';
	import type { OffsetPage } from '$lib/library';

	let {
		page,
		noun = 'item',
		plural = `${noun}s`,
		onpage
	}: {
		page: OffsetPage | null;
		/** Singular noun for the total, e.g. `book`. */
		noun?: string;
		plural?: string;
		onpage: (page: number) => void;
	} = $props();
</script>

{#if page}
	<div class="flex flex-wrap items-center gap-3 text-sm text-muted-foreground">
		<span>{countNoun(page.totalItems, noun, plural)}</span>
		{#if page.totalPages > 1}
			<div class="ml-auto flex items-center gap-2">
				<Button
					size="sm"
					variant="outline"
					aria-label="Previous page"
					disabled={page.currentPage <= 1}
					onclick={() => onpage(page.currentPage - 1)}
				>
					<ChevronLeftIcon />
				</Button>
				<span class="tabular-nums">Page {page.currentPage} of {page.totalPages}</span>
				<Button
					size="sm"
					variant="outline"
					aria-label="Next page"
					disabled={page.currentPage >= page.totalPages}
					onclick={() => onpage(page.currentPage + 1)}
				>
					<ChevronRightIcon />
				</Button>
			</div>
		{/if}
	</div>
{/if}
