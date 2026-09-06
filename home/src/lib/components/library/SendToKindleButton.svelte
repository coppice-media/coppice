<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Button } from '@stump/ui/components/ui/button';
	import * as Select from '@stump/ui/components/ui/select';
	import { request } from '@stump/ui/graphql/client';
	import {
		ConsoleKindleTargetsDocument,
		ConsoleSendToKindleDocument
	} from '$lib/graphql/generated/graphql';
	import { bytesLabel } from '$lib/format';

	let { mediaId }: { mediaId: string } = $props();

	// One shared cache entry for the whole page: every row resolves the
	// operator's Kindle devices from the same request.
	const targetsQuery = createQuery(() => ({
		queryKey: ['kindleTargets'],
		queryFn: () => request(ConsoleKindleTargetsDocument, {}),
		enabled: browser
	}));
	// A device is a target only once it carries an address; a revoked device is
	// still listed for its history and must not be one.
	const targets = $derived(
		(targetsQuery.data?.devices ?? []).filter((device) => device.kindleEmail && !device.revokedAt)
	);

	const send = createMutation(() => ({
		mutationFn: (deviceId: string) =>
			request(ConsoleSendToKindleDocument, { mediaId, deviceId }),
		onSuccess: ({ sendToKindle }) => {
			toast.success(
				`Sent ${sendToKindle.format.toUpperCase()} (${bytesLabel(sendToKindle.bytes)}) to ${sendToKindle.recipient}.`,
				// The note says why the book went unconverted — usually that the
				// server has no boko and Amazon converts the EPUB itself.
				{ description: sendToKindle.note ?? undefined }
			);
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to send the book.')
	}));
</script>

{#if targets.length === 1}
	<Button
		size="xs"
		variant="ghost"
		title={`Mail this book to ${targets[0].kindleEmail}`}
		onclick={() => send.mutate(targets[0].id)}
		disabled={send.isPending}
	>
		{send.isPending ? 'Sending…' : 'Send to Kindle'}
	</Button>
{:else if targets.length > 1}
	<Select.Root type="single" onValueChange={(value) => send.mutate(value)}>
		<Select.Trigger
			class="h-7 gap-1 border-0 px-2 text-xs shadow-none"
			aria-label="Send to Kindle"
			disabled={send.isPending}
		>
			{send.isPending ? 'Sending…' : 'Send to Kindle'}
		</Select.Trigger>
		<Select.Content>
			<Select.Group>
				<Select.Label>Send to</Select.Label>
				{#each targets as target (target.id)}
					<Select.Item value={target.id} label={`${target.name} · ${target.kindleEmail}`} />
				{/each}
			</Select.Group>
		</Select.Content>
	</Select.Root>
{/if}
