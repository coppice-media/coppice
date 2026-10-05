<script lang="ts" module>
	import type { ManagedUser } from '$lib/users';

	/** A server-owner action on one account, confirmed before it runs. */
	export interface UserAction {
		kind: 'lock' | 'unlock' | 'signOut' | 'delete' | 'purge';
		user: ManagedUser;
	}
</script>

<script lang="ts">
	/**
	 * Confirms and runs one owner action. A failure stays in the dialog next
	 * to the button that caused it; success closes it and refetches the list,
	 * so the row shows what the server stored.
	 */
	import { createMutation, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import * as AlertDialog from '@stump/ui/components/ui/alert-dialog';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import { request } from '@stump/ui/graphql/client';
	import { errorMessage } from '@stump/ui/utils/errors.js';
	import {
		UsersDeleteDocument,
		UsersRevokeSessionsDocument,
		UsersSetLockDocument
	} from '$lib/graphql/generated/graphql';
	import { countNoun } from '$lib/format';

	let {
		open = $bindable(false),
		action
	}: {
		open?: boolean;
		/** Kept by the parent after closing so the dialog can animate out intact. */
		action: UserAction | null;
	} = $props();

	let typedName = $state('');

	const copy = $derived.by(() => {
		if (!action) return null;
		const name = action.user.username;
		switch (action.kind) {
			case 'lock':
				return {
					title: `Lock ${name}?`,
					description: `${name} is signed out everywhere and every sign-in is refused until you unlock the account. Nothing else about it changes.`,
					confirm: 'Lock account',
					destructive: true
				};
			case 'unlock':
				return {
					title: `Unlock ${name}?`,
					description: `${name} can sign in again.`,
					confirm: 'Unlock account',
					destructive: false
				};
			case 'signOut':
				return {
					title: `Sign ${name} out everywhere?`,
					description: `Ends every signed-in session of this account (${countNoun(action.user.loginSessionsCount, 'active session')}). They can sign straight back in; lock the account to keep them out.`,
					confirm: 'Sign out everywhere',
					destructive: true
				};
			case 'delete':
				return {
					title: `Delete ${name}?`,
					description: `Stops ${name} from signing in and keeps the deleted account listed with its username reserved. Sessions, API/device tokens, and guest-reader links are revoked; pending club invitations and active shares are revoked, and reader sessions it created are closed. Reading history and other account data remain. There is no undo.`,
					confirm: 'Delete account',
					destructive: true
				};
			case 'purge':
				return {
					title: `Permanently delete ${name}?`,
					description: `Removes ${name}'s account, credentials, private reading data, personal collections, and requests. Shared library books are untouched. Public/shared reading lists and smart lists, server emoji, and closed club-reader history stay under the server owner's custody; other shared records may retain their content without the account link. Exported files, Git copies, and notifications already delivered outside Coppice are not removed. There is no undo.`,
					confirm: 'Delete permanently',
					destructive: true
				};
		}
	});

	const queryClient = useQueryClient();
	const run = createMutation(() => ({
		mutationFn: async ({ kind, user }: UserAction): Promise<string> => {
			switch (kind) {
				case 'lock':
				case 'unlock':
					await request(UsersSetLockDocument, { id: user.id, lock: kind === 'lock' });
					return kind === 'lock' ? `Locked ${user.username}.` : `Unlocked ${user.username}.`;
				case 'signOut': {
					const result = await request(UsersRevokeSessionsDocument, { id: user.id });
					return `Ended ${countNoun(result.deleteUserSessions, 'session')} of ${user.username}.`;
				}
				case 'delete':
				case 'purge':
					await request(UsersDeleteDocument, { id: user.id, hardDelete: kind === 'purge' });
					return kind === 'purge'
						? `Permanently deleted ${user.username}.`
						: `Deleted ${user.username}.`;
			}
		},
		onSuccess: (message) => {
			toast.success(message);
			void queryClient.invalidateQueries({ queryKey: ['users'] });
			open = false;
		}
	}));

	const confirmed = $derived(
		action !== null && (action.kind !== 'purge' || typedName === action.user.username)
	);
</script>

<AlertDialog.Root
	bind:open
	onOpenChangeComplete={(isOpen) => {
		if (isOpen) return;
		typedName = '';
		run.reset();
	}}
>
	<AlertDialog.Content>
		{#if action && copy}
			<AlertDialog.Header>
				<AlertDialog.Title>{copy.title}</AlertDialog.Title>
				<AlertDialog.Description>{copy.description}</AlertDialog.Description>
			</AlertDialog.Header>
			{#if action.kind === 'purge'}
				<div class="flex flex-col gap-2">
					<Label for="user-purge-confirm" class="block font-normal">
						Type <strong class="font-semibold">{action.user.username}</strong> to confirm.
					</Label>
					<Input
						id="user-purge-confirm"
						bind:value={typedName}
						autocomplete="off"
						spellcheck={false}
						disabled={run.isPending}
					/>
				</div>
			{/if}
			{#if run.isError}
				<Alert variant="destructive">
					<AlertTitle>{copy.confirm} failed</AlertTitle>
					<AlertDescription>{errorMessage(run.error)}</AlertDescription>
				</Alert>
			{/if}
			<AlertDialog.Footer>
				<AlertDialog.Cancel disabled={run.isPending}>Cancel</AlertDialog.Cancel>
				<AlertDialog.Action
					variant={copy.destructive ? 'destructive' : 'default'}
					disabled={!confirmed || run.isPending}
					onclick={() => {
						if (action && confirmed) run.mutate(action);
					}}
				>
					{run.isPending ? 'Working…' : copy.confirm}
				</AlertDialog.Action>
			</AlertDialog.Footer>
		{/if}
	</AlertDialog.Content>
</AlertDialog.Root>
