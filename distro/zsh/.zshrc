# Willie's own zsh configuration for a shell session (ZDOTDIR points
# here). History lives in the session's private home; the shell starts
# in the project's workspace and shows its branch in the prompt.
HISTFILE=$HOME/.zsh_history
HISTSIZE=5000
SAVEHIST=5000

[ -n "$WILLIE_WORKSPACE" ] && cd "$WILLIE_WORKSPACE" 2>/dev/null

autoload -Uz add-zsh-hook
_willie_branch() {
    WILLIE_BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null)
}
add-zsh-hook precmd _willie_branch

setopt PROMPT_SUBST
PROMPT='%F{114}%n@willie%f %F{75}%~%f %F{180}${WILLIE_BRANCH}%f ❯ '
