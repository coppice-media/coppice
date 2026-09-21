<script lang="ts" module>
	/**
	 * The shape of a TanStack query this component can drive directly. Kept
	 * structural so the package does not take a dependency on
	 * `@tanstack/svelte-query`, which each app installs itself.
	 */
	export type QueryStateQuery = {
		isPending: boolean;
		error: unknown;
		refetch: () => unknown;
	};
</script>

<script lang="ts">
	/**
	 * The four states every remote panel has — loading, failed, empty, loaded
	 * — resolved in one place, so a screen never invents its own third
	 * spinner or swallows an error into an empty list.
	 *
	 * Pass a query and it reads `isPending` / `error` / `refetch` off it;
	 * pass `pending` / `error` / `onretry` when the data comes from
	 * somewhere else (a subscription, a plain `fetch`, several queries
	 * folded together).
	 */
	import type { Snippet } from "svelte";
	import RefreshCwIcon from "@lucide/svelte/icons/refresh-cw";
	import TriangleAlertIcon from "@lucide/svelte/icons/triangle-alert";
	import { Alert, AlertAction, AlertDescription, AlertTitle } from "@stump/ui/components/ui/alert/index.js";
	import { Button } from "@stump/ui/components/ui/button/index.js";
	import { EmptyState } from "@stump/ui/components/ui/empty/index.js";
	import { Skeleton } from "@stump/ui/components/ui/skeleton/index.js";
	import { errorMessage } from "@stump/ui/utils/errors.js";
	import { cn } from "@stump/ui/utils.js";

	let {
		query,
		pending,
		error,
		onretry,
		empty = false,
		rows = 3,
		skeleton,
		emptyTitle = "Nothing here yet",
		emptyDescription,
		emptyIcon,
		emptyActions,
		errorTitle = "Something went wrong",
		class: className,
		children
	}: {
		query?: QueryStateQuery;
		/** Loading, when there is no `query` to read it from. */
		pending?: boolean;
		error?: unknown;
		onretry?: () => void;
		/** The query succeeded and returned nothing. */
		empty?: boolean;
		/** Placeholder rows the default skeleton draws. */
		rows?: number;
		skeleton?: Snippet;
		emptyTitle?: string;
		emptyDescription?: string;
		emptyIcon?: Snippet;
		emptyActions?: Snippet;
		errorTitle?: string;
		class?: string;
		children: Snippet;
	} = $props();

	const isPending = $derived(pending ?? query?.isPending ?? false);
	const failure = $derived(error ?? query?.error);
	const retry = $derived(onretry ?? (query ? () => void query.refetch() : undefined));
</script>

{#if isPending}
	<div data-slot="query-state" data-state="pending" class={cn("flex flex-col gap-3", className)}>
		{#if skeleton}
			{@render skeleton()}
		{:else}
			{#each { length: rows } as _, index (index)}
				<Skeleton class="h-10 w-full" />
			{/each}
		{/if}
	</div>
{:else if failure}
	<Alert data-slot="query-state" data-state="error" variant="destructive" class={className}>
		<TriangleAlertIcon aria-hidden="true" />
		<AlertTitle>{errorTitle}</AlertTitle>
		<AlertDescription>{errorMessage(failure)}</AlertDescription>
		{#if retry}
			<AlertAction>
				<Button variant="outline" size="sm" onclick={retry}>
					<RefreshCwIcon data-icon="inline-start" aria-hidden="true" />
					Retry
				</Button>
			</AlertAction>
		{/if}
	</Alert>
{:else if empty}
	<EmptyState
		data-state="empty"
		class={className}
		title={emptyTitle}
		description={emptyDescription}
		icon={emptyIcon}
	>
		{#if emptyActions}
			{@render emptyActions()}
		{/if}
	</EmptyState>
{:else}
	{@render children()}
{/if}
