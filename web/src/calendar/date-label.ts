const formats = new Map<string, Intl.DateTimeFormat>();
export function dateLabel(date: Date, options: Intl.DateTimeFormatOptions = { dateStyle: 'long' }) {
    const key = JSON.stringify(options, Object.keys(options).sort());
    let format = formats.get(key);
    if (!format) {
        format = new Intl.DateTimeFormat(undefined, options);
        if (formats.size >= 32) formats.delete(formats.keys().next().value!);
        formats.set(key, format);
    }
    return format.format(date);
}
