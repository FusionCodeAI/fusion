import React from "react";

export interface UserMessageProps {
  content: string;
}

export function UserMessage({ content }: UserMessageProps) {
  return (
    <div className="flex justify-end w-full py-1">
      <div className="max-w-[85%] md:max-w-[540px] bg-zinc-100 text-zinc-900 rounded-2xl px-4 py-2.5 text-[14px] leading-relaxed shadow-xs whitespace-pre-wrap break-words">
        {content}
      </div>
    </div>
  );
}

export default UserMessage;
