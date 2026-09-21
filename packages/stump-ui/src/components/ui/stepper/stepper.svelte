<script lang="ts" module>
	import { getContext, setContext } from "svelte";

	export type StepperOrientation = "horizontal" | "vertical";

	export type StepperContext = {
		/** The step being worked on, 1-based. */
		readonly current: number;
		readonly orientation: StepperOrientation;
	};

	const STEPPER_KEY = Symbol("stepper");

	export function setStepperCtx(ctx: StepperContext): void {
		setContext(STEPPER_KEY, ctx);
	}

	export function getStepperCtx(): StepperContext {
		const ctx = getContext<StepperContext | undefined>(STEPPER_KEY);
		if (!ctx) throw new Error("<Stepper.Item> must be used inside <Stepper.Root>");
		return ctx;
	}
</script>

<script lang="ts">
	/**
	 * Where the visitor is in a short linear flow. The current step is the
	 * only progress state the caller owns; each item derives complete /
	 * current / upcoming from it, so the steps cannot disagree with the
	 * screen they label.
	 */
	import type { HTMLOlAttributes } from "svelte/elements";
	import { cn, type WithElementRef } from "@stump/ui/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		current,
		orientation = "horizontal",
		children,
		...restProps
	}: WithElementRef<HTMLOlAttributes, HTMLOListElement> & {
		current: number;
		orientation?: StepperOrientation;
	} = $props();

	setStepperCtx({
		get current() {
			return current;
		},
		get orientation() {
			return orientation;
		}
	});
</script>

<ol
	bind:this={ref}
	data-slot="stepper"
	data-orientation={orientation}
	class={cn(
		"group/stepper flex w-full gap-2 data-[orientation=vertical]:flex-col data-[orientation=vertical]:gap-4",
		className
	)}
	{...restProps}
>
	{@render children?.()}
</ol>
