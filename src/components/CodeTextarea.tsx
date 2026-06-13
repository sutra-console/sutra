// A textarea with a left line-number gutter, for the macro editor. The gutter
// scrolls in lockstep with the textarea by translating its contents by the
// textarea's scrollTop. Forwards its ref to the underlying <textarea> so callers
// can still read selection / insert tokens.
import { forwardRef, useState } from "react";

import { cn } from "@/lib/utils";

type Props = React.ComponentProps<"textarea">;

export const CodeTextarea = forwardRef<HTMLTextAreaElement, Props>(
  ({ className, value, onScroll, ...props }, ref) => {
    const [scrollTop, setScrollTop] = useState(0);
    const lineCount = Math.max(1, String(value ?? "").split("\n").length);

    return (
      <div
        className={cn(
          "relative flex overflow-hidden rounded-md border border-input bg-transparent font-mono text-xs shadow-sm focus-within:ring-2 focus-within:ring-ring",
          className,
        )}
      >
        <div aria-hidden className="shrink-0 overflow-hidden border-r bg-muted/20 py-2 pl-2 pr-1.5 text-right">
          <div style={{ transform: `translateY(${-scrollTop}px)` }}>
            {Array.from({ length: lineCount }, (_, i) => (
              <div key={i} className="h-5 leading-5 tabular-nums text-muted-foreground/50">
                {i + 1}
              </div>
            ))}
          </div>
        </div>
        <textarea
          ref={ref}
          value={value}
          spellCheck={false}
          onScroll={(e) => {
            setScrollTop(e.currentTarget.scrollTop);
            onScroll?.(e);
          }}
          className="min-w-0 flex-1 resize-none bg-transparent px-2 py-2 leading-5 outline-none placeholder:text-muted-foreground"
          {...props}
        />
      </div>
    );
  },
);
CodeTextarea.displayName = "CodeTextarea";
