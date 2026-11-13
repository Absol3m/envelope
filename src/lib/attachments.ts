/** Types d'image dont l'aperçu est proposé (les mêmes que `PREVIEW_TYPES` côté moteur) ; le reste s'ouvre dans l'application du système. */
export const previewable = (mime: string) => /^image\/(png|jpeg|gif|webp|avif|bmp|svg\+xml)$/i.test(mime);
