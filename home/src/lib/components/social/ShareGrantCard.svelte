<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import XIcon from '@lucide/svelte/icons/x';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardFooter, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import type { SocialShareGrant } from '$lib/social';
	import { formatAuthors, scopeNames } from '$lib/social';

	interface Props {
		grant: SocialShareGrant;
		direction: 'incoming' | 'outgoing';
		busy?: boolean;
		onAccept?: (grant: SocialShareGrant) => void;
		onDecline?: (grant: SocialShareGrant) => void;
		onRevoke?: (grant: SocialShareGrant) => void;
	}

	let { grant, direction, busy = false, onAccept, onDecline, onRevoke }: Props = $props();

	const canRespond = $derived(direction === 'incoming' && grant.state === 'PENDING');
	const canRevoke = $derived(
		direction === 'outgoing' && (grant.state === 'PENDING' || grant.state === 'ACTIVE')
	);
	const stateLabel = $derived(grant.state.replaceAll('_', ' '));
	const labels = $derived(scopeNames(grant.scopes));
</script>

<Card class="flex h-full flex-col">
	<CardHeader class="space-y-1">
		<div class="flex flex-wrap items-center justify-between gap-2">
			<CardTitle class="text-base">{grant.title}</CardTitle>
			<Badge variant={grant.state === 'ACTIVE' ? 'default' : grant.state === 'PENDING' ? 'outline' : 'secondary'}>
				{stateLabel}
			</Badge>
		</div>
		<p class="text-sm text-muted-foreground">{formatAuthors(grant.authors) || 'Author not provided'}</p>
	</CardHeader>
	<CardContent class="flex flex-1 flex-col gap-3 text-sm">
		<div>
			<p class="font-medium">Shared fields</p>
			{#if labels.length > 0}
				<ul class="mt-2 flex flex-wrap gap-2" aria-label="Granted sharing scopes">
					{#each labels as label (label)}
						<li><Badge variant="outline">{label}</Badge></li>
					{/each}
				</ul>
			{:else}
				<p class="mt-1 text-muted-foreground">No fields are shared.</p>
			{/if}
		</div>
		{#if grant.expiresAt}
			<p class="text-xs text-muted-foreground">Expires {new Date(grant.expiresAt).toLocaleDateString()}</p>
		{/if}
		<p class="text-xs text-muted-foreground">
			A share grant never includes a locator, device identifier, session timing, or raw sync payload.
		</p>
	</CardContent>
	{#if canRespond || canRevoke}
		<CardFooter class="flex flex-wrap gap-2 border-t pt-4">
			{#if canRespond}
				<Button type="button" size="sm" disabled={busy} onclick={() => onAccept?.(grant)}>
					<CheckIcon class="mr-1 size-4" />
					Accept these scopes
				</Button>
				<Button type="button" size="sm" variant="outline" disabled={busy} onclick={() => onDecline?.(grant)}>
					<XIcon class="mr-1 size-4" />
					Decline
				</Button>
			{:else}
				<Button type="button" size="sm" variant="outline" disabled={busy} onclick={() => onRevoke?.(grant)}>
					<RotateCcwIcon class="mr-1 size-4" />
					Revoke sharing
				</Button>
			{/if}
		</CardFooter>
	{/if}
</Card>
