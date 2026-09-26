<script lang="ts">
	/**
	 * One tile of the dashboard grid: a card with a title, an optional lede,
	 * a *See all* link into the full screen, and its own loading / error /
	 * empty state, so every widget resolves independently and an empty query
	 * never blanks a neighbour.
	 */
	import type { Snippet } from 'svelte';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import { Button } from '@stump/ui/components/ui/button';
	import {
		Card,
		CardAction,
		CardContent,
		CardDescription,
		CardHeader,
		CardTitle
	} from '@stump/ui/components/ui/card';
	import { QueryState } from '@stump/ui/components/ui/query-state';
	import { cn } from '@stump/ui/utils.js';

	let {
		title,
		description,
		href,
		hrefLabel = 'See all',
		query,
		empty = false,
		rows = 3,
		emptyTitle = 'Nothing here yet',
		emptyDescription,
		errorTitle = 'Unable to load this widget',
		class: className,
		action,
		skeleton,
		emptyActions,
		children
	}: {
		title: string;
		description?: string;
		/** The screen this widget summarises. */
		href?: string;
		hrefLabel?: string;
		query?: { isPending: boolean; error: unknown; refetch: () => unknown };
		empty?: boolean;
		rows?: number;
		emptyTitle?: string;
		emptyDescription?: string;
		errorTitle?: string;
		/** Grid placement (`@lg/page:col-span-7`) and anything else the tile needs. */
		class?: string;
		/** A control beside the link: a span selector, a metric toggle. */
		action?: Snippet;
		skeleton?: Snippet;
		emptyActions?: Snippet;
		children: Snippet;
	} = $props();
</script>

<Card class={className}>
	<CardHeader>
		<CardTitle class="@max-md/card-header:col-span-2">{title}</CardTitle>
		{#if description}
			<CardDescription class="@max-md/card-header:col-span-2">{description}</CardDescription>
		{/if}
		{#if action || href}
			<CardAction
				class={cn(
					'flex flex-wrap items-center gap-2 @max-md/card-header:col-span-2 @max-md/card-header:col-start-1 @max-md/card-header:row-span-1 @max-md/card-header:mt-1 @max-md/card-header:justify-self-start',
					description ? '@max-md/card-header:row-start-3' : '@max-md/card-header:row-start-2'
				)}
			>
				{#if action}{@render action()}{/if}
				{#if href}
					<Button {href} size="sm" variant="ghost" class="-mr-2 text-muted-foreground @max-md/card-header:-ml-2 @max-md/card-header:mr-0">
						{hrefLabel}
						<ArrowRightIcon data-icon="inline-end" aria-hidden="true" />
					</Button>
				{/if}
			</CardAction>
		{/if}
	</CardHeader>
	<CardContent class="@container/widget">
		<QueryState {query} {empty} {rows} {emptyTitle} {emptyDescription} {errorTitle} {skeleton} {emptyActions}>
			{@render children()}
		</QueryState>
	</CardContent>
</Card>
