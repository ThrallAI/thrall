// Markdown from Thrall (replies, plans, command results). Raw HTML is never rendered; links open
// in a new tab; code blocks get a Copy button.

import { memo, useRef, type ReactNode } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

import { CopyButton } from "./CopyButton";

function CodeBlock({ children }: { children?: ReactNode }) {
  const pre = useRef<HTMLPreElement>(null);
  return (
    <div className="th-code">
      <CopyButton text={() => pre.current?.innerText ?? ""} />
      <pre ref={pre}>{children}</pre>
    </div>
  );
}

const components: Components = {
  a: ({ href, children }) => (
    <a href={href} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  ),
  pre: ({ children }) => <CodeBlock>{children}</CodeBlock>,
};

export const Markdown = memo(function Markdown({ text }: { text: string }) {
  return (
    <div className="md">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {text}
      </ReactMarkdown>
    </div>
  );
});
