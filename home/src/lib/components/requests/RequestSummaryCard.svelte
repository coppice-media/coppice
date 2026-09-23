<script lang="ts">
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import Clock3Icon from '@lucide/svelte/icons/clock-3';
	import BookOpenIcon from '@lucide/svelte/icons/book-open';
	import { resolve } from '$app/paths';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import RequestStatusBadge from './RequestStatusBadge.svelte';
	import { relativeTime } from '$lib/format';
	import { statusDescription, safeText } from '$lib/requests';

	type RequestSummary = {
		id: string;
		title: string;
		authors?: string | null;
		coverUrl?: string | null;
		sourceProvider?: string | null;
		status: string;
		destinationShelfId?: string | null;
		destinationDeviceId?: string | null;
		createdAt: string;
		updatedAt: string;
		failureMessage?: string | null;
	};

	let { request }: { request: RequestSummary } = $props();
	const cover = $derived(request.coverUrl && /^https?:\/\//i.test(request.coverUrl) ? request.coverUrl : null);
	const origin = $derived(request.sourceProvider ? `External · ${safeText(request.sourceProvider, 80)}` : 'From this library');
</script>

<Card class="flex min-h-52 flex-col">
	<CardHeader class="flex-row items-start gap-3 space-y-0">
		{#if cover}
			<img src={cover} alt="" class="size-14 shrink-0 rounded-md border object-cover" loading="lazy" />
		{:else}
			<div class="flex size-14 shrink-0 items-center justify-center rounded-md border bg-muted/40" aria-hidden="true">
				<BookOpenIcon class="size-5 text-muted-foreground" />
			</div>
		{/if}
		<div class="min-w-0 flex-1">
			<div class="flex flex-wrap items-start gap-2">
				<CardTitle class="line-clamp-2 text-base">{request.title}</CardTitle>
				<RequestStatusBadge status={request.status} />
			</div>
			<p class="mt-1 text-sm text-muted-foreground">
				{request.authors || 'Author not provided'} · {origin}
			</p>
		</div>
	</CardHeader>
	<CardContent class="flex flex-1 flex-col gap-3 pt-0">
		<p class="text-sm text-muted-foreground">{request.failureMessage || statusDescription(request.status)}</p>
		<div class="mt-auto flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
			<span class="flex items-center gap-1"><Clock3Icon class="size-3.5" aria-hidden="true" /> Updated {relativeTime(request.updatedAt)}</span>
			{#if request.destinationShelfId || request.destinationDeviceId}
				<Badge variant="outline">Destination saved</Badge>
			{/if}
			<Button class="ml-auto" variant="ghost" size="sm" href={`${resolve('/requests')}/${request.id}`}>
				Open <ArrowRightIcon data-icon="inline-end" aria-hidden="true" />
			</Button>
		</div>
	</CardContent>
</Card>
