import { Children, type ReactNode } from 'react'

export function AnimatedText({ children, className = '' }: { children?: ReactNode; className?: string }) {
  return (
    <span className={`animated-word ${className}`}>
      {Children.map(children, (child) => typeof child === 'string'
        ? Array.from(child).map((letter, index) => <span className="headline-letter" key={index} style={{ animationDelay: `${index * 80}ms` }}>{letter === ' ' ? '\u00a0' : letter}</span>)
        : child)}
    </span>
  )
}
