export interface ApiResponse<T> {
  data: T
  message?: string
}

export interface ApiError {
  message: string
  code?: string
  status?: number
}

export interface PaginationParams {
  page?: number
  per_page?: number
}

export interface AuthCredentials {
  username: string
  password: string
}

export interface AuthUser {
  username: string
  role: string
}

export interface AuthSession {
  token: string
  expires_at: string
  username?: string
  role?: string
  user?: AuthUser
}
