function Assert-CompatibilityIdentity($Certificate,[string]$Subject,[string]$Thumbprint) {
    if($Certificate.Subject -cne $Subject -or $Certificate.Thumbprint -ine $Thumbprint){
        throw 'Unexpected compatibility certificate identity.'
    }
}

function Assert-CompatibilityCertificate($Certificate,[string]$Subject,[string]$Thumbprint) {
    Assert-CompatibilityIdentity $Certificate $Subject $Thumbprint
    $now=[DateTime]::UtcNow
    $from=$Certificate.NotBefore.ToUniversalTime()
    $until=$Certificate.NotAfter.ToUniversalTime()
    if($Certificate.Issuer -cne $Subject -or $from -gt $now -or $until -le $now -or ($until-$from).TotalDays -gt 3){
        throw 'Invalid compatibility certificate issuer or validity period.'
    }
    $usage=$Certificate.Extensions['2.5.29.15']
    $eku=$Certificate.Extensions['2.5.29.37']
    $constraints=$Certificate.Extensions['2.5.29.19']
    if(-not $usage -or $usage.KeyUsages -ne 'DigitalSignature' -or -not $constraints -or $constraints.CertificateAuthority -or
       -not $eku -or $eku.EnhancedKeyUsages.Count -ne 1 -or $eku.EnhancedKeyUsages[0].Value -cne '1.3.6.1.5.5.7.3.3'){
        throw 'Only a code-signing leaf certificate is allowed.'
    }
}

function Add-CompatibilityTrust($Store,$Certificate,[string]$Subject,[string]$Thumbprint) {
    Assert-CompatibilityCertificate $Certificate $Subject $Thumbprint
    if($Certificate.HasPrivateKey){throw 'Trust requires a public certificate.'}
    if($Store.Certificates.Find('FindByThumbprint',$Thumbprint,$false).Count){throw 'Certificate already trusted; refusing to take ownership.'}
    try {
        $Store.Add($Certificate)
        if($Store.Certificates.Find('FindByThumbprint',$Thumbprint,$false).Count -ne 1){throw 'Temporary trust was not confirmed.'}
    } catch {
        $Store.Remove($Certificate)
        throw
    }
}

function Assert-CompatibilityPackage([IO.Compression.ZipArchive]$Package,[string]$Name,[string]$Publisher,[string]$Version) {
    $manifests=@($Package.Entries | Where-Object FullName -IEQ 'AppxManifest.xml')
    if($manifests.Count -ne 1 -or $manifests[0].Length -gt 131072){throw 'Expected a single compatibility manifest.'}
    $settings=[Xml.XmlReaderSettings]::new()
    $settings.DtdProcessing='Prohibit'
    $settings.MaxCharactersInDocument=131072
    $inputStream=$manifests[0].Open()
    try {
        $reader=[Xml.XmlReader]::Create($inputStream,$settings)
        try {$doc=[Xml.XmlDocument]::new();$doc.XmlResolver=$null;$doc.Load($reader)} finally {$reader.Dispose()}
    } finally {$inputStream.Dispose()}
    $ns=[Xml.XmlNamespaceManager]::new($doc.NameTable)
    $ns.AddNamespace('p','http://schemas.microsoft.com/appx/manifest/foundation/windows10')
    $identities=$doc.SelectNodes('/p:Package/p:Identity',$ns)
    if($identities.Count -ne 1 -or $identities[0].GetAttribute('Name') -cne $Name -or
       $identities[0].GetAttribute('Publisher') -cne $Publisher -or $identities[0].GetAttribute('Version') -cne $Version){
        throw 'Package manifest does not match the compatibility test identity, publisher and version.'
    }
}
