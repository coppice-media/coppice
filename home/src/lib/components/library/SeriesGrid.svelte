<script lang="ts">
	import { resolve } from '$app/paths';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Cover } from '@stump/ui/components/ui/cover';
	import { Progress } from '@stump/ui/components/ui/progress';
	import { countNoun } from '$lib/format';
	import type { ConsoleSeriesCardFragment } from '$lib/graphql/generated/graphql';

	let { series }: { series: ConsoleSeriesCardFragment[] } = $props();
</script>

<ul class="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4">
	{#each series as entry (entry.id)}
		<li class="flex flex-col overflow-hidden rounded-xl border bg-card">
			<a
				class="flex flex-1 flex-col gap-2"
				href={resolve('/(app)/series/[id]', { id: entry.id })}
				aria-label={entry.resolvedName}
			>
				<Cover src={entry.thumbnail.url} class="w-full" />
				<div class="flex flex-1 flex-col gap-1 px-3 pb-3">
					<span class="line-clamp-2 text-sm font-medium">{entry.resolvedName}</span>
					<span class="text-xs text-muted-foreground">
						{countNoun(entry.mediaCount, 'book')} · {entry.unreadCount} unread
					</span>
					{#if entry.status !== 'READY'}
						<Badge variant="destructive" class="w-fit">{entry.status}</Badge>
					{/if}
					<div class="mt-auto pt-2">
						<Progress value={Math.round(entry.percentageCompleted)} />
					</div>
				</div>
			</a>
		</li>
	{/each}
</ul>
