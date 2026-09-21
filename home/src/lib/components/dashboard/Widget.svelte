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
		<CardTitle>{title}</CardTitle>
		{#if description}
			<CardDescription>{description}</CardDescription>
		{/if}
		{#if action || href}
			<CardAction class="flex items-center gap-2">
				{#if action}{@render action()}{/if}
				{#if href}
					<Button {href} size="sm" variant="ghost" class="-mr-2 text-muted-foreground">
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
