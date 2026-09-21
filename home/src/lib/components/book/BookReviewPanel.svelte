<script lang="ts">
	import MessageSquareTextIcon from '@lucide/svelte/icons/message-square-text';
	import SaveIcon from '@lucide/svelte/icons/save';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import { Alert, AlertDescription } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Checkbox } from '@stump/ui/components/ui/checkbox';
	import { Label } from '@stump/ui/components/ui/label';
	import { Textarea } from '@stump/ui/components/ui/textarea';
	import type { BookReview } from '$lib/book/detail';

	let {
		review,
		canEdit = true,
		busy = false,
		onsave,
		ondelete
	}: {
		review?: BookReview | null;
		canEdit?: boolean;
		busy?: boolean;
		onsave: (input: { rating: number; content: string; isPrivate: boolean }) => void | Promise<void>;
		ondelete: () => void | Promise<void>;
	} = $props();

	let rating = $state(0);
	let content = $state('');
	let isPrivate = $state(false);
	let error = $state('');
	let loadedReviewId = $state<string | null>(null);

	$effect(() => {
		const reviewId = review?.id ?? null;
		if (reviewId === loadedReviewId) return;
		loadedReviewId = reviewId;
		rating = review?.rating ?? 0;
		content = review?.content ?? '';
		isPrivate = review?.isPrivate ?? false;
	});


	async function save() {
		if (rating < 1 || rating > 5) {
			error = 'Choose a rating from 1 to 5.';
			return;
		}
		error = '';
		await onsave({ rating, content: content.trim(), isPrivate });
	}
</script>

<Card>
	<CardHeader>
		<div class="flex flex-wrap items-start gap-3">
			<div class="mr-auto">
				<CardTitle class="flex items-center gap-2"><MessageSquareTextIcon class="size-4" aria-hidden="true" />Your review</CardTitle>
				<CardDescription>Keep a private note or share a rating with other readers.</CardDescription>
			</div>
			{#if review}<Badge variant={review.isPrivate ? 'outline' : 'secondary'}>{review.isPrivate ? 'Private' : 'Shared'}</Badge>{/if}
		</div>
	</CardHeader>
	<CardContent class="flex flex-col gap-4">
		{#if error}<Alert variant="destructive"><AlertDescription>{error}</AlertDescription></Alert>{/if}
		<div class="grid gap-2 sm:max-w-xs">
			<Label for="book-review-rating">Rating</Label>
			<select id="book-review-rating" class="h-9 rounded-md border bg-background px-3 text-sm" bind:value={rating} disabled={!canEdit || busy}>
				<option value={0}>Choose a rating</option>
				{#each [1, 2, 3, 4, 5] as value}<option value={value}>{value} / 5</option>{/each}
			</select>
		</div>
		<div class="grid gap-2">
			<Label for="book-review-content">Note</Label>
			<Textarea id="book-review-content" rows={5} placeholder="What should you remember about this book?" bind:value={content} disabled={!canEdit || busy} />
		</div>
		<div class="flex flex-wrap items-center gap-3">
			<Label class="flex items-center gap-2 text-sm font-normal"><Checkbox bind:checked={isPrivate} disabled={!canEdit || busy} />Keep this review private</Label>
			<div class="ml-auto flex gap-2">
				{#if review && canEdit}<Button variant="ghost" size="sm" onclick={ondelete} disabled={busy}><Trash2Icon data-icon="inline-start" aria-hidden="true" />Delete</Button>{/if}
				<Button size="sm" onclick={save} disabled={!canEdit || busy}><SaveIcon data-icon="inline-start" aria-hidden="true" />{busy ? 'Saving…' : 'Save review'}</Button>
			</div>
		</div>
		{#if !canEdit}<p class="text-xs text-muted-foreground">Review editing is unavailable for this account.</p>{/if}
	</CardContent>
</Card>
