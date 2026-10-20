(permission-api-authorization-for-a-user-principal-authorization-for-a-user-principal)=
# Authorization for a user principal

```{image} images/user-interaction.png
:alt: Permission Service-User Interaction.png
```

Applications that make API requests to Omniverse services that require authentication must integrate with an **Identity Provider** 
to authenticate users. OpenID Connect is the default mechanism; an implementation MAY use other authentication mechanisms instead.

When an application calls an **Omniverse Service** on behalf of the user, it must pass authentication information received from the **Identity Provider** within the request. 
The **Omniverse Service** may call the **Permission Service** to verify the user identity and check if the user is authorized to call the API. 
The **Permission Service** retrieves public keys from the **Identity Provider** and validates the user information. 
If it’s valid, the **Permission Service** uses the underlying authorization system to verify that the user can perform the specified action in the system.

The sequence diagram below demonstrates how the **Customer Application** can use Authorization Flow with PKCE ([RFC 7636](https://www.rfc-editor.org/rfc/rfc7636)) 
to authenticate the user with OpenID Connect and call an Omniverse Service:

```{image} images/pkce.png
:alt: Permission Service - PKCE.png
```

(permission-api-authorization-for-a-user-principal-see-next)=
## See next

* {ref}`Authorization for a service principal <permission-api-authorization-for-a-service-principal-authorization-for-a-service-principal>`