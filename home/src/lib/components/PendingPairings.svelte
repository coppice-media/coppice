<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Input } from '@stump/ui/components/ui/input';
	import { request } from '@stump/ui/graphql/client';
	import {
		ApproveDevicePairingDocument,
		DenyDevicePairingDocument,
		PendingDevicePairingsDocument
	} from '$lib/graphql/generated/graphql';
	import { DEVICE_KIND_LABELS } from '$lib/devices';
	import { relativeTime } from '$lib/format';

	let { now }: { now: Date } = $props();

	const queryClient = useQueryClient();
	const pairings = createQuery(() => ({
		queryKey: ['pendingDevicePairings'],
		queryFn: () => request(PendingDevicePairingsDocument, {}),
		enabled: browser,
		// Pairings expire in minutes; keep the list honest without a subscription.
		refetchInterval: 10_000
	}));
	const pending = $derived(
		(pairings.data?.pendingDevicePairings ?? []).filter((pairing) => pairing.status === 'PENDING')
	);

	let codes = $state<Record<string, string>>({});

	function settle(): void {
		void queryClient.invalidateQueries({ queryKey: ['pendingDevicePairings'] });
		void queryClient.invalidateQueries({ queryKey: ['devices'] });
	}

	const approve = createMutation(() => ({
		mutationFn: (pairingId: string) =>
			request(ApproveDevicePairingDocument, { pairingId, code: codes[pairingId]?.trim() ?? '' }),
		onSuccess: () => {
			toast.success('Pairing approved; the device picks up its credential on its next poll.');
			settle();
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'The pairing code was not accepted.')
	}));
	const deny = createMutation(() => ({
		mutationFn: (pairingId: string) => request(DenyDevicePairingDocument, { pairingId }),
		onSuccess: settle,
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to deny the pairing.')
	}));
</script>

{#if pending.length}
	<section class="flex flex-col gap-3" aria-labelledby="pending-pairings-heading">
		<h2 id="pending-pairings-heading" class="text-lg font-semibold tracking-tight">
			Waiting for approval
		</h2>
		<p class="text-sm text-muted-foreground">
			A device on your network asked to pair. Type the code it shows to approve it.
		</p>
		<ul class="flex flex-col gap-2">
			{#each pending as pairing (pairing.id)}
				<li class="flex flex-wrap items-center gap-3 rounded-lg border bg-card p-3 text-sm">
					<Badge variant="secondary">{DEVICE_KIND_LABELS[pairing.kind]}</Badge>
					<span class="font-medium">{pairing.name ?? 'Unnamed device'}</span>
					<span class="text-muted-foreground">from {pairing.remoteIp}</span>
					<span class="text-muted-foreground">
						expires {relativeTime(pairing.expiresAt, now)}
					</span>
					<form
						class="ml-auto flex items-center gap-2"
						onsubmit={(event) => {
							event.preventDefault();
							approve.mutate(pairing.id);
						}}
					>
						<Input
							class="h-8 w-28 font-mono"
							inputmode="numeric"
							pattern="[0-9]{6}"
							maxlength={6}
							placeholder="000000"
							aria-label="Pairing code"
							bind:value={codes[pairing.id]}
							required
						/>
						<Button type="submit" size="sm" disabled={approve.isPending}>Approve</Button>
						<Button
							type="button"
							size="sm"
							variant="ghost"
							onclick={() => deny.mutate(pairing.id)}
							disabled={deny.isPending}
						>
							Deny
						</Button>
					</form>
				</li>
			{/each}
		</ul>
	</section>
{/if}
