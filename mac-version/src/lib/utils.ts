import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** Deterministic gradient pair per genre so related worlds share a
 *  visual family (fantasy → purple-blue, sci-fi → cyan-teal, etc.). */
const GENRE_GRADIENTS: Record<string, string> = {
  fantasy: "from-purple-500/20 to-blue-500/20",
  scifi: "from-cyan-500/20 to-teal-500/20",
  manga: "from-violet-500/20 to-fuchsia-500/20",
  romance: "from-pink-500/20 to-rose-500/20",
  horror: "from-red-500/20 to-orange-500/20",
  mystery: "from-indigo-500/20 to-violet-500/20",
  adventure: "from-amber-500/20 to-yellow-500/20",
  drama: "from-slate-500/20 to-gray-500/20",
  comedy: "from-lime-500/20 to-green-500/20",
};

export function genreGradient(genre: string): string {
  const key = genre.toLowerCase().replace(/[\s/]+/g, "_");
  return GENRE_GRADIENTS[key] ?? "from-[var(--color-accent)]/20 to-[var(--color-accent)]/5";
}

/** Version used on card backgrounds (more saturated). */
export function genreGradientCard(genre: string): string {
  const base = genreGradient(genre);
  return base.replace("/20", "/30");
}
