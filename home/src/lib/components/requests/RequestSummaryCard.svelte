<script lang="ts">
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import Clock3Icon from '@lucide/svelte/icons/clock-3';
	import { resolve } from '$app/paths';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardFooter } from '@stump/ui/components/ui/card';
	import { Cover } from '@stump/ui/components/ui/cover';
	import RequestStatusBadge from './RequestStatusBadge.svelte';
	import { absoluteTime, relativeTime } from '$lib/format';
	import { providerLabel, requestFormatLabel, safeCoverUrl } from '$lib/requests';

	type RequestSummary = {
		id: string;
		title: string;
		authors?: string | null;
		coverUrl?: string | null;
		sourceProvider?: string | null;
		status: string;
		format: string;
		isbn?: string | null;
		preferredNarrator?: string | null;
		destinationShelfId?: string | null;
		destinationDeviceId?: string | null;
		createdAt: string;
		updatedAt: string;
		failureMessage?: string | null;
	};

	let { request }: { request: RequestSummary } = $props();
	const cover = $derived(safeCoverUrl(request.coverUrl));
	const origin = $derived(providerLabel(request.sourceProvider) || 'This library');
	const href = $derived(resolve('/(app)/requests/[id]', { id: request.id }));
</script>

<Card size="sm" class="gap-3">
	<CardContent class="flex-row gap-3">
		<Cover src={cover} class="h-18 w-12 rounded-md border" />
		<div class="flex min-w-0 flex-1 flex-col gap-1.5">
			<div class="flex items-start justify-between gap-2">
				<a {href} class="truncate font-medium hover:underline" title={request.title}>{request.title}</a>
				<RequestStatusBadge status={request.status} />
			</div>
			<p class="truncate text-xs text-muted-foreground">
				{request.authors || 'Author not provided'} · {origin}
			</p>
			<div class="flex flex-wrap items-center gap-1.5">
				<Badge variant="outline">{requestFormatLabel(request.format)}</Badge>
				{#if request.isbn}
					<Badge variant="outline" class="font-mono">{request.isbn}</Badge>
				{/if}
				{#if request.preferredNarrator}
					<Badge variant="outline">Narrator: {request.preferredNarrator}</Badge>
				{/if}
				{#if request.destinationShelfId || request.destinationDeviceId}
					<Badge variant="outline">Destination saved</Badge>
				{/if}
			</div>
		</div>
	</CardContent>
	<CardFooter class="justify-between gap-2 text-xs text-muted-foreground">
		<span class="flex items-center gap-1" title={absoluteTime(request.updatedAt)}>
			<Clock3Icon class="size-3.5" aria-hidden="true" />
			Updated {relativeTime(request.updatedAt)}
		</span>
		<Button variant="ghost" size="sm" class="-mr-2" {href}>
			Open <ArrowRightIcon data-icon="inline-end" aria-hidden="true" />
		</Button>
	</CardFooter>
</Card>
