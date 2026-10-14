import { Link } from "react-router-dom";
import { Screen } from "../components/ui";

export default function NotFoundScreen() {
  return (
    <Screen title="Nothing here">
      <p className="text-mist">This page isn’t part of the field.</p>
      <Link className="btn btn-primary self-start" to="/">
        Back to the field
      </Link>
    </Screen>
  );
}
