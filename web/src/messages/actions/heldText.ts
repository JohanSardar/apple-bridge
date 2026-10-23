function comparableText(text: string): string {
    return text.replace(/\uFFFC/g, '').replace(/\r\n?|\u2028/g, '\n').normalize('NFC').trim();
}

export function matchesHeldText(held: string | undefined, echo: string | undefined): boolean {
    return held === undefined || comparableText(held) === comparableText(echo ?? '');
}
