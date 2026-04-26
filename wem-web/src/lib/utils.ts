import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"

export const API_BASE_URL = import.meta.env.VITE_API_BASE_URL ?? 'http://localhost:6809/api/v1'

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}
