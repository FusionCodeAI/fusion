import React from "react";

export interface UserMessageProps {
  content: string;
}

export function UserMessage({ content }: UserMessageProps) {
  return (
    <div
      data-testid="user-message"
      className="w-full relative p-3 my-1.5 rounded-xl bg-zinc-100/90 text-zinc-900 text-[13px] font-sans leading-relaxed whitespace-pre-wrap break-words border border-zinc-200/50 select-text"
    >
      {content}
    </div>
  );
}

export default UserMessage;
