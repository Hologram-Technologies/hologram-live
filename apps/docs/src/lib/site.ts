const base = import.meta.env.BASE_URL.replace(/\/$/, '');

export function sitePath(path: string) {
  const absolutePath = path.startsWith('/') ? path : `/${path}`;
  return `${base}${absolutePath}`;
}

export function withoutBase(path: string) {
  if (base !== '' && (path === base || path.startsWith(`${base}/`))) {
    return path.slice(base.length) || '/';
  }
  return path;
}
