; Hooks do instalador NSIS (per-machine, roda elevado). $INSTDIR contém o sidecar relay-service.exe.

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
