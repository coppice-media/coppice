<script lang="ts">
	import CrownIcon from '@lucide/svelte/icons/crown';
	import LogOutIcon from '@lucide/svelte/icons/log-out';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import UserMinusIcon from '@lucide/svelte/icons/user-minus';
	import UserPlusIcon from '@lucide/svelte/icons/user-plus';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Input } from '@stump/ui/components/ui/input';
	import type { BookClub, BookClubInvitation } from '$lib/social';

	interface Props {
		clubs: readonly BookClub[];
		invitations: readonly BookClubInvitation[];
		canCreate: boolean;
		busy?: boolean;
		error?: string | null;
		onInvite?: (clubId: string, userId: string, role: string) => void;
		onRemove?: (clubId: string, memberId: string) => void;
		onLeave?: (clubId: string) => void;
		onRespondInvitation?: (invitation: BookClubInvitation, accept: boolean) => void;
		onCreate?: (input: { name: string; description: string; isPrivate: boolean; creatorHideProgress: boolean }) => void;
	}

	let {
		clubs,
		invitations,
		canCreate,
		busy = false,
		error = null,
		onInvite,
		onRemove,
		onLeave,
		onRespondInvitation,
		onCreate
	}: Props = $props();

	let selectedClubId = $state<string | null>(null);
	let inviteUserId = $state('');
	let inviteRole = $state('MEMBER');
	let createName = $state('');
	let createDescription = $state('');
	let createPrivate = $state(true);
	let creatorHideProgress = $state(true);

	const selectedClub = $derived(
		clubs.find((club) => club.id === selectedClubId) ?? clubs[0] ?? null
	);
	const membershipRole = $derived(selectedClub?.membership?.role ?? '');
	const canManageMembers = $derived(membershipRole === 'ADMIN' || membershipRole === 'CREATOR');
	const canLeave = $derived(Boolean(selectedClub?.membership) && !selectedClub?.membership?.isCreator);

	function submitInvite(): void {
		const userId = inviteUserId.trim();
		if (!selectedClub || !canManageMembers || !userId) return;
		onInvite?.(selectedClub.id, userId, inviteRole);
		inviteUserId = '';
	}

	function submitCreate(): void {
		const name = createName.trim();
		if (!canCreate || !name) return;
		onCreate?.({
			name,
			description: createDescription.trim(),
			isPrivate: createPrivate,
			creatorHideProgress
		});
		createName = '';
		createDescription = '';
		createPrivate = true;
		creatorHideProgress = true;
	}
</script>

<div class="flex flex-col gap-4">
	{#if error}
		<Alert variant="destructive">
			<AlertTitle>BookClub action failed</AlertTitle>
			<AlertDescription>{error}</AlertDescription>
		</Alert>
	{/if}

	{#if invitations.length > 0}
		<Card>
			<CardHeader>
				<CardTitle class="text-base">Invitations</CardTitle>
				<CardDescription>Accept only clubs you recognize. Joining a club does not grant cross-user reading access.</CardDescription>
			</CardHeader>
			<CardContent class="flex flex-col gap-3">
				{#each invitations as invitation (invitation.id)}
					<div class="flex flex-wrap items-center justify-between gap-3 rounded-md border p-3">
						<div>
							<p class="font-medium">BookClub invitation</p>
							<p class="text-sm text-muted-foreground">Role: {invitation.role.toLowerCase()}</p>
						</div>
						<div class="flex flex-wrap gap-2">
							<Button type="button" size="sm" disabled={busy} onclick={() => onRespondInvitation?.(invitation, true)}>Accept</Button>
							<Button type="button" size="sm" variant="outline" disabled={busy} onclick={() => onRespondInvitation?.(invitation, false)}>Decline</Button>
						</div>
					</div>
				{/each}
			</CardContent>
		</Card>
	{/if}

	<div class="grid gap-4 @2xl/page:grid-cols-[minmax(14rem,18rem)_1fr]">
		<Card>
			<CardHeader>
				<CardTitle class="text-base">Your BookClubs</CardTitle>
				<CardDescription>Membership is explicit and separate from sharing grants.</CardDescription>
			</CardHeader>
			<CardContent class="flex flex-col gap-2">
				{#if clubs.length === 0}
					<p class="text-sm text-muted-foreground">You are not a member of a BookClub yet.</p>
				{:else}
					{#each clubs as club (club.id)}
						<button
							type="button"
							class="flex items-center justify-between rounded-md border px-3 py-2 text-left text-sm transition-colors hover:bg-accent {selectedClub?.id === club.id ? 'bg-accent' : ''}"
							aria-pressed={selectedClub?.id === club.id}
							onclick={() => (selectedClubId = club.id)}
						>
							<span class="flex min-w-0 items-center gap-2">
								{#if club.emoji}<span aria-hidden="true">{club.emoji}</span>{/if}
								<span class="truncate">{club.name}</span>
							</span>
							<Badge variant="outline">{club.membersCount}</Badge>
						</button>
					{/each}
				{/if}
			</CardContent>
		</Card>

		{#if selectedClub}
			<Card>
				<CardHeader>
					<div class="flex flex-wrap items-start justify-between gap-3">
						<div>
							<CardTitle class="text-base">{selectedClub.name}</CardTitle>
							<CardDescription>{selectedClub.description || 'No description provided.'}</CardDescription>
						</div>
						<Badge variant="outline">{membershipRole.toLowerCase() || 'member'}</Badge>
					</div>
				</CardHeader>
				<CardContent class="flex flex-col gap-4">
					<div>
						<h3 class="text-sm font-medium">Members</h3>
						<ul class="mt-2 divide-y rounded-md border" aria-label="BookClub members">
							{#each selectedClub.members as member (member.id)}
								<li class="flex flex-wrap items-center justify-between gap-3 px-3 py-2 text-sm">
									<div class="flex min-w-0 items-center gap-2">
										{#if member.isCreator}<CrownIcon class="size-4 shrink-0" aria-label="Creator" />{/if}
										<span class="truncate">{member.displayName || member.username}</span>
										<Badge variant="secondary">{member.role.toLowerCase()}</Badge>
									</div>
									{#if canManageMembers && !member.isCreator && member.id !== selectedClub.membership?.id}
										<Button type="button" size="sm" variant="ghost" disabled={busy} aria-label="Remove {member.username}" onclick={() => onRemove?.(selectedClub.id, member.id)}>
											<UserMinusIcon class="size-4" />
										</Button>
									{/if}
								</li>
							{/each}
						</ul>
						<p class="mt-2 text-xs text-muted-foreground">Member progress remains hidden unless the member separately opts in through a scoped share grant.</p>
					</div>

					{#if canManageMembers}
						<div class="rounded-md border p-3">
							<h3 class="text-sm font-medium">Invite a member</h3>
							<p class="mt-1 text-xs text-muted-foreground">Use the recipient's user ID from the narrow picker. Never paste a device or session identifier.</p>
							<div class="mt-3 grid gap-2 @sm/page:grid-cols-[1fr_auto]">
								<label class="sr-only" for="bookclub-invite-user">User ID</label>
								<Input id="bookclub-invite-user" bind:value={inviteUserId} placeholder="User ID" autocomplete="off" />
								<select class="h-9 rounded-md border bg-background px-3 text-sm" bind:value={inviteRole} aria-label="Invitation role">
									<option value="MEMBER">Member</option>
									<option value="MODERATOR">Moderator</option>
									<option value="ADMIN">Admin</option>
								</select>
							</div>
							<Button type="button" size="sm" class="mt-3" disabled={busy || !inviteUserId.trim()} onclick={submitInvite}>
								<UserPlusIcon class="mr-1 size-4" />
								Send invitation
							</Button>
						</div>
					{/if}
				</CardContent>
				{#if canLeave}
					<CardFooter class="border-t pt-4">
						<Button type="button" size="sm" variant="outline" disabled={busy} onclick={() => onLeave?.(selectedClub.id)}>
							<LogOutIcon class="mr-1 size-4" />
							Leave BookClub
						</Button>
					</CardFooter>
				{/if}
			</Card>
		{:else}
			<Card>
				<CardHeader>
					<CardTitle class="text-base">Start a BookClub</CardTitle>
					<CardDescription>{canCreate ? 'Create a private reading group and invite members explicitly.' : 'Ask a server owner for the Create BookClub permission.'}</CardDescription>
				</CardHeader>
				{#if canCreate}
					<CardContent class="flex flex-col gap-3">
						<div class="grid gap-2 @sm/page:grid-cols-2">
							<div>
								<label class="text-sm font-medium" for="bookclub-name">Name</label>
								<Input id="bookclub-name" class="mt-1" bind:value={createName} placeholder="Weekend reading" />
							</div>
							<div>
								<label class="text-sm font-medium" for="bookclub-description">Description</label>
								<Input id="bookclub-description" class="mt-1" bind:value={createDescription} placeholder="What are you reading?" />
							</div>
						</div>
						<label class="flex items-center gap-2 text-sm">
							<input type="checkbox" bind:checked={createPrivate} />
							<span>Keep this club private</span>
						</label>
						<label class="flex items-center gap-2 text-sm">
							<input type="checkbox" bind:checked={creatorHideProgress} />
							<span>Keep my progress hidden</span>
						</label>
					</CardContent>
					<CardFooter class="border-t pt-4">
						<Button type="button" size="sm" disabled={busy || !createName.trim()} onclick={submitCreate}>
							<PlusIcon class="mr-1 size-4" />
							Create BookClub
						</Button>
					</CardFooter>
				{/if}
			</Card>
		{/if}
	</div>
</div>
