<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import {
		Card,
		CardContent,
		CardDescription,
		CardHeader,
		CardTitle
	} from '@stump/ui/components/ui/card';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';

	// Every dashboard widget loads independently, so each owns its own
	// skeleton / error / empty state instead of one page-wide gate.
	let {
		title,
		description,
		pending = false,
		error = null,
		errorTitle,
		empty = false,
		emptyTitle,
		emptyDescription,
		skeletonRows = 2,
		action,
		children
	}: {
		title: string;
		description?: string;
		pending?: boolean;
		error?: unknown;
		errorTitle: string;
		empty?: boolean;
		emptyTitle: string;
		emptyDescription?: string;
		skeletonRows?: number;
		action?: Snippet;
		children: Snippet;
	} = $props();

	const message = $derived(
		error instanceof Error ? error.message : error ? 'The server rejected the request.' : null
	);
</script>

<Card>
	<CardHeader class="flex flex-wrap items-start gap-3">
		<div class="mr-auto">
			<CardTitle class="text-base">{title}</CardTitle>
			{#if description}
				<CardDescription>{description}</CardDescription>
			{/if}
		</div>
		{#if action}{@render action()}{/if}
	</CardHeader>
	<CardContent>
		{#if pending}
			<div class="flex flex-col gap-2">
				{#each { length: skeletonRows } as _, row (row)}
					<Skeleton class="h-10 w-full" />
				{/each}
			</div>
		{:else if error}
			<Alert variant="destructive">
				<AlertTitle>{errorTitle}</AlertTitle>
				<AlertDescription>{message}</AlertDescription>
			</Alert>
		{:else if empty}
			<Empty class="border-none py-6">
				<EmptyHeader>
					<EmptyTitle>{emptyTitle}</EmptyTitle>
					{#if emptyDescription}
						<EmptyDescription>{emptyDescription}</EmptyDescription>
					{/if}
				</EmptyHeader>
			</Empty>
		{:else}
			{@render children()}
		{/if}
	</CardContent>
</Card>
