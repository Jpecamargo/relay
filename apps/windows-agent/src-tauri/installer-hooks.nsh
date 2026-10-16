; Hooks do instalador NSIS (per-machine, roda elevado). $INSTDIR contém o sidecar relay-service.exe.

!macro NSIS_HOOK_PREINSTALL
  ; Update: para o serviço e encerra o helper (mesmo exe, --session-helper) antes de sobrescrever
  ; relay-service.exe em uso. Primeira instalação: os comandos falham e o erro é ignorado.
  nsExec::ExecToLog 'sc.exe stop RelayService'
  Pop $0
  nsExec::ExecToLog 'taskkill.exe /F /IM relay-service.exe'
  Pop $0
  ; Dá tempo ao SCM/Windows de soltar o arquivo.
  Sleep 1500
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Serviço RelayService (SYSTEM) + regras de firewall do Relay (somente perfil privado).
  nsExec::ExecToLog '"$INSTDIR\relay-service.exe" install'
  Pop $0
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Relay"'
  Pop $0
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Relay" dir=in action=allow protocol=TCP localport=47800,47804 profile=private'
  Pop $0
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Relay" dir=in action=allow protocol=UDP localport=47801-47803 profile=private'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Para o serviço antes de o uninstall apagar o executável; erros são ignorados (já removido).
  nsExec::ExecToLog 'sc.exe stop RelayService'
  Pop $0
  nsExec::ExecToLog '"$INSTDIR\relay-service.exe" uninstall'
  Pop $0
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Relay"'
  Pop $0
!macroend
