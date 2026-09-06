<script lang="ts">
	import { resolve } from '$app/paths';
	import { createMutation, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import {
		Card,
		CardContent,
		CardDescription,
		CardFooter,
		CardHeader,
		CardTitle
	} from '@stump/ui/components/ui/card';
	import { request } from '@stump/ui/graphql/client';
	import {
		ConsoleAnalyzeLibraryDocument,
		ConsoleScanLibraryDocument,
		type ConsoleLibraryCardFragment
	} from '$lib/graphql/generated/graphql';
	import { bytesLabel, countLabel, relativeTime } from '$lib/format';
	import { LIBRARY_TYPE_LABELS } from '$lib/library';

	let { library }: { library: ConsoleLibraryCardFragment } = $props();

	const queryClient = useQueryClient();
	const scan = createMutation(() => ({
		mutationFn: () => request(ConsoleScanLibraryDocument, { id: library.id }),
		onSuccess: () => {
			toast.success(`Scanning ${library.name}.`);
			void queryClient.invalidateQueries({ queryKey: ['libraries'] });
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to start the scan.')
	}));
	const analyze = createMutation(() => ({
		mutationFn: () => request(ConsoleAnalyzeLibraryDocument, { id: library.id }),
		onSuccess: () => toast.success(`Analyzing the books of ${library.name}.`),
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to start the analysis.')
	}));

	const detail = $derived(resolve('/(app)/library/[id]', { id: library.id }));
</script>

<Card>
	<CardHeader>
		<CardTitle class="flex items-center gap-2">
			{#if library.emoji}
				<span aria-hidden="true">{library.emoji}</span>
			{/if}
			<a class="hover:underline" href={detail}>{library.name}</a>
			{#if library.status !== 'READY'}
				<Badge variant="destructive">{library.status}</Badge>
			{/if}
			{#if library.sourceProvider}
				<Badge variant="secondary">Provider</Badge>
			{/if}
		</CardTitle>
		<CardDescription class="truncate" title={library.path}>
			{LIBRARY_TYPE_LABELS[library.config.libraryType]} · {library.path}
		</CardDescription>
	</CardHeader>
	<CardContent class="grid grid-cols-2 gap-3 text-sm sm:grid-cols-4">
		{#each [['Series', countLabel(library.stats.seriesCount)], ['Books', countLabel(library.stats.bookCount)], ['Finished', countLabel(library.stats.completedBooks)], ['On disk', bytesLabel(library.stats.totalBytes)]] as [label, value] (label)}
			<div>
				<div class="text-muted-foreground">{label}</div>
				<div class="text-lg font-medium tabular-nums">{value}</div>
			</div>
		{/each}
		<p class="col-span-2 text-muted-foreground sm:col-span-4">
			Last scanned {relativeTime(library.lastScannedAt)}{library.config.watch
				? ' · watching for changes'
				: ''}
		</p>
	</CardContent>
	<CardFooter class="flex flex-wrap gap-2">
		<Button size="sm" variant="outline" href={detail}>Browse</Button>
		<Button size="sm" variant="outline" onclick={() => scan.mutate()} disabled={scan.isPending}>
			{scan.isPending ? 'Scanning…' : 'Scan'}
		</Button>
		<Button
			size="sm"
			variant="outline"
			onclick={() => analyze.mutate()}
			disabled={analyze.isPending}
		>
			{analyze.isPending ? 'Analyzing…' : 'Analyze'}
		</Button>
	</CardFooter>
</Card>
