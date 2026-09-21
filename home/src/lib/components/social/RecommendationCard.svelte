<script lang="ts">
	import ArrowDownToLineIcon from '@lucide/svelte/icons/arrow-down-to-line';
	import CheckIcon from '@lucide/svelte/icons/check';
	import Clock3Icon from '@lucide/svelte/icons/clock-3';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import XIcon from '@lucide/svelte/icons/x';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardFooter, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import type { AdaptiveRecommendation, SocialRecommendation } from '$lib/social';
	import { formatAuthors, recommendationReason } from '$lib/social';

	interface Props {
		recommendation: SocialRecommendation;
		direction: 'incoming' | 'outgoing';
		adaptive?: readonly AdaptiveRecommendation[];
		busy?: boolean;
		requestSubmitted?: boolean;
		onAccept?: (recommendation: SocialRecommendation) => void;
		onDecline?: (recommendation: SocialRecommendation) => void;
		onDismiss?: (recommendation: SocialRecommendation) => void;
		onRevoke?: (recommendation: SocialRecommendation) => void;
		onRequest?: (recommendation: SocialRecommendation) => void;
	}

	let {
		recommendation,
		direction,
		adaptive = [],
		busy = false,
		requestSubmitted = false,
		onAccept,
		onDecline,
		onDismiss,
		onRevoke,
		onRequest
	}: Props = $props();


	const isExternal = $derived(recommendation.targetKind === 'EXTERNAL_WORK');
	const stateLabel = $derived(recommendation.state.replaceAll('_', ' '));
	const authors = $derived(formatAuthors(recommendation.authors));
	const reason = $derived(recommendationReason(recommendation, adaptive));
	const canRespond = $derived(direction === 'incoming' && recommendation.state === 'PENDING');
	const canRevoke = $derived(direction === 'outgoing' && recommendation.state === 'PENDING');
	const canDismiss = $derived(direction === 'incoming' && recommendation.state === 'ACCEPTED');
	const canQueue = $derived(
		direction === 'incoming' &&
		recommendation.state === 'ACCEPTED' &&
		!recommendation.requestId &&
		!requestSubmitted &&
		(isExternal || recommendation.targetKind === 'INTERNAL_MEDIA' || recommendation.targetKind === 'INTERNAL_WORK')
	);
</script>

<Card class="flex h-full flex-col">
	<CardHeader class="flex-row items-start gap-3 space-y-0">
		{#if recommendation.coverUrl}
			<img
				class="size-14 shrink-0 rounded-md border object-cover"
				src={recommendation.coverUrl}
				alt="Cover for {recommendation.title}"
				loading="lazy"
			/>
		{:else}
			<div class="flex size-14 shrink-0 items-center justify-center rounded-md border bg-muted text-muted-foreground" aria-hidden="true">
				<ExternalLinkIcon class="size-5" />
			</div>
		{/if}
		<div class="min-w-0 flex-1">
			<div class="flex flex-wrap items-center gap-2">
				<CardTitle class="text-base">{recommendation.title}</CardTitle>
				<Badge variant={recommendation.state === 'PENDING' ? 'outline' : recommendation.state === 'ACCEPTED' ? 'default' : 'secondary'}>
					{stateLabel}
				</Badge>
			</div>
			<p class="mt-1 text-sm text-muted-foreground">{authors || 'Author not provided'}</p>
		</div>
	</CardHeader>
	<CardContent class="flex flex-1 flex-col gap-3 text-sm">
		<div class="rounded-md border border-dashed bg-muted/30 px-3 py-2">
			<p class="font-medium">Why this is here</p>
			<p class="mt-1 text-muted-foreground">{reason}</p>
		</div>
		{#if recommendation.message}
			<p class="text-muted-foreground">“{recommendation.message}”</p>
		{/if}
		<div class="flex flex-wrap gap-2 text-xs text-muted-foreground">
			{#if isExternal}
				<Badge variant="outline">Not in your library</Badge>
			{:else}
				<Badge variant="outline">In library</Badge>
			{/if}
			{#if recommendation.handoffState === 'REQUESTED'}
				<Badge variant="outline">Request started</Badge>
			{:else if recommendation.handoffState === 'LINKED'}
				<Badge variant="outline">Request linked</Badge>
			{:else if recommendation.handoffState === 'FAILED'}
				<Badge variant="destructive">Request failed</Badge>
			{/if}
		</div>
	</CardContent>
	{#if canRespond || canRevoke || canDismiss || canQueue}
		<CardFooter class="flex flex-wrap gap-2 border-t pt-4">
			{#if canRespond}
				<Button type="button" size="sm" disabled={busy} onclick={() => onAccept?.(recommendation)}>
					<CheckIcon class="mr-1 size-4" />
					Accept
				</Button>
				<Button type="button" size="sm" variant="outline" disabled={busy} onclick={() => onDecline?.(recommendation)}>
					<XIcon class="mr-1 size-4" />
					Decline
				</Button>
			{:else if canRevoke}
				<Button type="button" size="sm" variant="outline" disabled={busy} onclick={() => onRevoke?.(recommendation)}>
					<RotateCcwIcon class="mr-1 size-4" />
					Revoke
				</Button>
			{:else if canQueue}
				<Button type="button" size="sm" disabled={busy} onclick={() => onRequest?.(recommendation)}>
					<ArrowDownToLineIcon class="mr-1 size-4" />
					{isExternal ? 'Request this book' : 'Queue this title'}
				</Button>
			{/if}
			{#if canDismiss}
				<Button type="button" size="sm" variant="ghost" disabled={busy} onclick={() => onDismiss?.(recommendation)}>
					<Clock3Icon class="mr-1 size-4" />
					Dismiss
				</Button>
			{/if}
		</CardFooter>
	{/if}
</Card>
