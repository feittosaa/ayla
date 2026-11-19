import os
import secrets
from fastapi import Header, HTTPException

def verify_token(x_ayla_token: str = Header(None)):
    app_token = os.getenv("AYLA_TOKEN")
    if not app_token or not x_ayla_token or not secrets.compare_digest(
        x_ayla_token.encode(), app_token.encode()
    ):
        raise HTTPException(status_code=401, detail="Unauthorized")
